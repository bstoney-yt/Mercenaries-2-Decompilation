"""Side-by-side clips: m2-alpha ("Ours") next to the 360 game in Xenia, one case per fix.

    python -I clips.py capture <case.json> --exe <m2-alpha.exe> --install <dir> [--out <root>]
    python -I clips.py compose <case.json> --ours <clip|frames dir> --xenia <clip> [--out <dir>]
                               [--ours-start S] [--xenia-start S] [--ours-offset S] [--xenia-offset S]
    python -I clips.py notes <case.json> [--out <dir>]
    python -I clips.py check <case.json>

`capture` waits for the single GPU slot and refuses to run without it (the
lock file `C:\\Claude\\scratch\\gpu.lock`, shared with the nightly runner, and
no `m2-alpha*`/`xenia*` program running), plays the case's Xenia route while
recording Xenia's window (falling back to the route's own screenshots when the
recording comes out black, as from a disconnected session), then plays
m2-alpha offscreen with dense `--mission-shots`, lines both up on the case's
start events and composes `side-by-side.mp4` (plus `.gif`, `thumb.png`,
`notes.md`). `compose` does the last step alone from two existing clips: no
GPU, no game. Only the standard library and a local ffmpeg are used.

Clips and frames stay where `--out` points (the LAN): nothing is uploaded.
See docs/side-by-side-clips.md for the case format.
"""

from __future__ import annotations

import argparse
import ctypes
import json
import os
import re
import subprocess
import sys
import threading
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
FFMPEG = os.environ.get(
    "M2_FFMPEG", r"C:\Claude\scratch\tools\ffmpeg\ffmpeg-9.0.2-essentials_build\bin\ffmpeg.exe"
)
LOCK = Path(os.environ.get("M2_GPU_LOCK", r"C:\Claude\scratch\gpu.lock"))
BUSY = ("m2-alpha", "xenia")
FONT = "C\\:/Windows/Fonts/arial.ttf"
# m2-alpha names its --mission-shots to a tenth of a second (`-t005.1`).
MAX_OURS_FPS = 10.0

UV_WITH = [
    "pillow", "numpy", "winrt-Windows.Media.Ocr", "winrt-Windows.Graphics.Imaging",
    "winrt-Windows.Storage", "winrt-Windows.Storage.Streams", "winrt-Windows.Foundation",
    "winrt-Windows.Foundation.Collections", "winrt-Windows.Globalization",
]


class CaseError(ValueError):
    pass


# ---- the case file -----------------------------------------------------------

DEFAULTS = {
    "fps": 30,
    "size": "640x360",
    "slowmo": 1.0,
    "swap": False,
    "labels": ["Ours", "Xenia"],
}


def load_case(path: Path) -> dict:
    case = json.loads(Path(path).read_text(encoding="utf-8-sig"))
    return check_case(case, Path(path).parent)


def check_case(case: dict, base: Path = HERE) -> dict:
    """Fills defaults and checks a case; raises CaseError naming what is wrong."""
    c = {**DEFAULTS, **case}
    for k in ("name", "caption", "duration", "ours", "xenia"):
        if k not in c:
            raise CaseError(f"case: missing {k!r}")
    if not re.fullmatch(r"[A-Za-z0-9._-]+", str(c["name"])):
        raise CaseError("case: name must be letters, digits, . _ -")
    if not (0 < float(c["duration"]) <= 300):
        raise CaseError("case: duration must be 0-300 s")
    if not re.fullmatch(r"\d+x\d+", c["size"]):
        raise CaseError("case: size must be WxH")
    w, h = (int(v) for v in c["size"].split("x"))
    if w % 2 or h % 2:
        raise CaseError("case: size must be even")
    if not (0.1 <= float(c["slowmo"]) <= 10):
        raise CaseError("case: slowmo must be 0.1-10 (2 = half speed)")
    o, x = c["ours"], c["xenia"]
    o.setdefault("offset", 0.0)
    x.setdefault("offset", 0.0)
    x.setdefault("capture", "video")
    x.setdefault("args", ["--headless=true"])
    x.setdefault("audio", True)
    if x["capture"] not in ("video", "frames"):
        raise CaseError("case: xenia.capture is video or frames")
    o.setdefault("args", [])
    o.setdefault("wall", 1800)
    shots = o.get("shots")
    if shots is not None:
        if float(shots.get("fps", 0)) <= 0 or float(shots["fps"]) > MAX_OURS_FPS:
            raise CaseError(f"case: ours.shots.fps must be 0-{MAX_OURS_FPS:g} (shot names resolve 0.1 s)")
        if float(shots["to"]) <= float(shots["from"]):
            raise CaseError("case: ours.shots.to must be after from")
    for side, key in ((o, "ours"), (x, "xenia")):
        parse_start(side.get("start", "t:0"), key)
    routes = x.get("routes", [])
    for r in routes:
        if not (base / r).exists() and not (HERE / "routes" / r).exists():
            raise CaseError(f"case: route file {r} not found")
    return c


def parse_start(spec: str, side: str):
    """`t:<seconds>`, `mark:<label>` (Xenia route mark), `shot:<name>` (Xenia
    shot), `log:<regex>` (m2-alpha stderr; group 1 is seconds of play)."""
    kind, _, value = str(spec).partition(":")
    if kind == "t":
        return ("t", float(value))
    if kind in ("mark", "shot") and side == "xenia" and value:
        return (kind, value)
    if kind == "log" and side == "ours" and value:
        rx = re.compile(value)
        if rx.groups < 1:
            raise CaseError("case: ours log: start needs a (seconds) group")
        return ("log", rx)
    raise CaseError(f"case: {side}.start {spec!r} is not one of t:, " + ("mark:, shot:" if side == "xenia" else "log:"))


# ---- logs and times ------------------------------------------------------------

def parse_route_log(text: str) -> dict:
    """xc.py's route.log: `<t> <message>` lines, t seconds since its t0."""
    out = {"t0": None, "window": None, "marks": [], "shots": []}
    for line in text.splitlines():
        m = re.match(r"\s*([\d.]+) (.*)$", line)
        if not m:
            continue
        t, msg = float(m.group(1)), m.group(2)
        if msg.startswith("t0 epoch "):
            out["t0"] = float(msg.split()[2])
        elif msg.startswith("window 0x") and out["window"] is None:
            out["window"] = int(msg.split()[1], 16)
        elif msg.startswith("mark "):
            out["marks"].append((t, msg[5:].strip()))
        elif msg.startswith("shot ") and not msg.endswith("FAILED") and "unreadable" not in msg:
            out["shots"].append((t, msg.split()[1]))
    return out


def parse_m2_shots(text: str) -> list[tuple[float, str]]:
    """`mission-shot: <s> s: <path>` lines of m2-alpha's stderr."""
    shots = []
    for line in text.splitlines():
        m = re.match(r"mission-shot: ([\d.]+) s: (.+)$", line.strip())
        if m:
            shots.append((float(m.group(1)), m.group(2).strip()))
    return shots


def ffmpeg_start_epoch(stderr: str) -> float | None:
    """gdigrab stamps frames with the wall clock: its `start:` is an epoch."""
    m = re.search(r"start: (\d{9,}\.\d+)", stderr)
    return float(m.group(1)) if m else None


def event_time(spec: str, side: str, route: dict | None = None, m2_err: str = "") -> float:
    """The start event's time on that side's own clock: Xenia's route
    seconds, or m2-alpha's seconds of play."""
    kind, v = parse_start(spec, side)
    if kind == "t":
        return v
    if kind == "mark":
        for t, label in route["marks"]:
            if label == v or label.startswith(v):
                return t
        raise CaseError(f"xenia: no route mark {v!r}")
    if kind == "shot":
        for t, name in route["shots"]:
            if name == v or name == v + ".png":
                return t
        raise CaseError(f"xenia: no route shot {v!r}")
    for line in m2_err.splitlines():
        m = v.search(line)
        if m:
            return float(m.group(1))
    raise CaseError(f"ours: no stderr line matches {v.pattern!r}")


def mission_shots(shots: dict) -> str:
    """The `--mission-shots` list for {from, to, fps}, to a tenth of a second."""
    a, b, fps = float(shots["from"]), float(shots["to"]), float(shots["fps"])
    n = round((b - a) * fps)
    seen, out = set(), []
    for i in range(n + 1):
        s = round(a + i / fps, 1)
        if s not in seen:
            seen.add(s)
            out.append(f"{s:g}")
    return ",".join(out)


# ---- ffmpeg -------------------------------------------------------------------

def ff(args: list[str], **kw) -> subprocess.CompletedProcess:
    p = subprocess.run([FFMPEG, "-hide_banner", "-loglevel", "error", "-y", *args],
                       capture_output=True, text=True, check=False, **kw)
    if p.returncode != 0:
        raise RuntimeError(f"ffmpeg failed: {p.stderr.strip()[-800:]}")
    return p


def frames_to_clip(frames: list[tuple[float, Path]], out: Path, fps: int) -> float:
    """A constant-rate clip from timed stills (each held until the next);
    returns the first still's time, the clip's time 0."""
    frames = sorted((t, Path(p)) for t, p in frames if Path(p).exists())
    if not frames:
        raise CaseError(f"no frames for {out.name}")
    lst = out.with_suffix(".frames.txt")
    lines = []
    for i, (t, p) in enumerate(frames):
        nxt = frames[i + 1][0] if i + 1 < len(frames) else t + 1.0 / fps
        lines.append(f"file '{p.resolve().as_posix()}'")
        lines.append(f"duration {max(nxt - t, 1.0 / fps):.4f}")
    lines.append(f"file '{frames[-1][1].resolve().as_posix()}'")
    lst.write_text("\n".join(lines) + "\n", encoding="utf-8")
    ff(["-f", "concat", "-safe", "0", "-i", str(lst), "-vf", f"fps={fps},format=yuv420p",
        "-c:v", "libx264", "-crf", "18", "-preset", "veryfast", str(out)])
    return frames[0][0]


def text_file(d: Path, name: str, text: str) -> str:
    p = d / name
    p.write_text(text, encoding="utf-8")
    # Filter-graph escaping of a Windows path: C\:/x/y.
    return p.resolve().as_posix().replace(":", "\\:")


def side_filter(i: int, start: float, case: dict, label_file: str) -> str:
    """Input i trimmed from `start` for the case's duration, slowed, sized
    and labelled. A negative start pads with the first frame."""
    w, h = case["size"].split("x")
    d = float(case["duration"])
    slow = float(case["slowmo"])
    fps = int(case["fps"])
    pre = f"tpad=start_duration={-start:.3f}:start_mode=clone," if start < 0 else ""
    s = max(start, 0.0)
    return (
        f"[{i}:v]{pre}trim=start={s:.3f}:duration={d:.3f},setpts=(PTS-STARTPTS)*{slow:g},"
        f"fps={fps},scale={w}:{h}:force_original_aspect_ratio=decrease,"
        f"pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1,"
        f"drawtext=fontfile='{FONT}':expansion=none:textfile='{label_file}':x=12:y=10:fontsize={max(int(h) // 18, 12)}:"
        f"fontcolor=white:box=1:boxcolor=black@0.55:boxborderw=6[v{i}]"
    )


def compose(case: dict, ours: Path, xenia: Path, ours_start: float, xenia_start: float, out: Path) -> dict:
    """Ours and Xenia side by side (Ours left unless `swap`), one MP4 with a
    caption bar, a GIF and a thumbnail. Starts are seconds into each clip
    (plus each side's offset)."""
    out.mkdir(parents=True, exist_ok=True)
    o0 = ours_start + float(case["ours"].get("offset", 0.0))
    x0 = xenia_start + float(case["xenia"].get("offset", 0.0))
    labels = case["labels"]
    inputs = [ours, xenia]
    starts = [o0, x0]
    names = [labels[0], labels[1]]
    if case["swap"]:
        inputs.reverse()
        starts.reverse()
        names.reverse()
    w, h = (int(v) for v in case["size"].split("x"))
    bar = max(h // 8, 24)
    cap = text_file(out, "caption.txt", case["caption"])
    graph = ";".join([
        side_filter(0, starts[0], case, text_file(out, "label0.txt", names[0])),
        side_filter(1, starts[1], case, text_file(out, "label1.txt", names[1])),
        (
            f"[v0][v1]hstack=inputs=2,pad=iw:ih+{bar}:0:0:black,"
            f"drawtext=fontfile='{FONT}':expansion=none:textfile='{cap}':x=12:y=h-{bar}+({bar}-text_h)/2:"
            f"fontsize={max(bar // 2, 11)}:fontcolor=white[out]"
        ),
    ])
    mp4 = out / "side-by-side.mp4"
    ff(["-i", str(inputs[0]), "-i", str(inputs[1]), "-filter_complex", graph, "-map", "[out]",
        "-an", "-c:v", "libx264", "-crf", "20", "-preset", "medium", "-pix_fmt", "yuv420p",
        "-movflags", "+faststart", str(mp4)])
    gif = out / "side-by-side.gif"
    ff(["-i", str(mp4), "-vf",
        f"fps=12,scale={min(2 * w, 960)}:-2:flags=lanczos,split[a][b];[a]palettegen[p];[b][p]paletteuse",
        str(gif)])
    thumb = out / "thumb.png"
    mid = float(case["duration"]) * float(case["slowmo"]) / 2
    ff(["-ss", f"{mid:.3f}", "-i", str(mp4), "-frames:v", "1", str(thumb)])
    meta = {"case": case["name"], "ours": str(ours), "xenia": str(xenia),
            "ours_start": o0, "xenia_start": x0, "swap": case["swap"],
            "fps": case["fps"], "size": case["size"], "slowmo": case["slowmo"],
            "duration": case["duration"]}
    (out / "compose.json").write_text(json.dumps(meta, indent=2), encoding="utf-8")
    (out / "notes.md").write_text(notes(case), encoding="utf-8")
    return meta


def notes(case: dict) -> str:
    """The release-notes snippet: caption, the fix, and where the clip's
    link goes once Brendan has uploaded it (nothing is uploaded here)."""
    fix = case.get("fix", "")
    left, right = (case["labels"][1], case["labels"][0]) if case["swap"] else tuple(case["labels"])
    lines = [f"### {case['caption']}", ""]
    if case.get("summary"):
        lines += [case["summary"], ""]
    if fix:
        lines += [f"Fixed in {fix}.", ""]
    lines.append(f"[Side by side: {left} (left), {right} (right; the Xbox 360 game in Xenia)](CLIP_LINK)"
                 if right == "Xenia" else f"[Side by side: {left} (left), {right} (right)](CLIP_LINK)")
    lines += ["", f"<!-- clip: side-by-side.mp4 of case {case['name']}; replace CLIP_LINK after uploading -->", ""]
    return "\n".join(lines)


# ---- the GPU slot ---------------------------------------------------------------

def processes() -> list[tuple[str, int]]:
    if os.name != "nt":
        return []
    p = subprocess.run(["tasklist", "/FO", "CSV", "/NH"], capture_output=True, text=True, check=False)
    out = []
    for line in p.stdout.splitlines():
        f = line.strip().strip('"').split('","')
        if len(f) >= 2 and f[1].isdigit():
            out.append((f[0], int(f[1])))
    return out


def busy(procs, prefixes=BUSY):
    return [(n, p) for n, p in procs if n.lower().startswith(tuple(prefixes))]


def lock_stale(owner: dict, procs, now: float, stale_s: float = 8 * 3600) -> bool:
    alive = any(p == owner.get("pid") for _, p in procs)
    return not alive or now - float(owner.get("started_unix", 0)) > stale_s


def take_lock(path: Path, purpose: str):
    """The lock file (the nightly runner's format: pid, started_unix,
    purpose), or None with who holds it."""
    for _ in range(2):
        try:
            fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
        except FileExistsError:
            try:
                owner = json.loads(path.read_text(encoding="utf-8"))
            except (OSError, ValueError):
                owner = {"pid": 0, "started_unix": 0, "purpose": "unreadable lock"}
            if lock_stale(owner, processes(), time.time()):
                path.unlink(missing_ok=True)
                continue
            return None, owner
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump({"pid": os.getpid(), "started_unix": int(time.time()), "purpose": purpose}, f)
        return path, None
    return None, {"purpose": "contended"}


def wait_for_slot(purpose: str, wait_minutes: float, checks: int = 3, poll: float = 20.0):
    """Takes the lock and sees the slot free for `checks` checks in a row,
    or gives up after `wait_minutes`: then the capture refuses to run."""
    end = time.time() + wait_minutes * 60
    while True:
        lock, owner = take_lock(LOCK, purpose)
        if lock:
            free = 0
            while free < checks:
                held = busy(processes())
                free = free + 1 if not held else 0
                if free >= checks:
                    return lock
                if time.time() > end:
                    lock.unlink(missing_ok=True)
                    return None
                print(f"clips: GPU slot held by {held}; waiting" if held else "clips: slot free, checking again", flush=True)
                time.sleep(poll)
        if time.time() > end:
            print(f"clips: {LOCK} held by {owner}", flush=True)
            return None
        time.sleep(poll)


# ---- capture ----------------------------------------------------------------------

def window_title(hwnd: int) -> str:
    u = ctypes.WinDLL("user32")
    buf = ctypes.create_unicode_buffer(512)
    u.GetWindowTextW(hwnd, buf, 512)
    return buf.value


def mostly_black(clip: Path) -> bool:
    p = subprocess.run([FFMPEG, "-hide_banner", "-i", str(clip), "-vf", "blackdetect=d=0.5:pix_th=0.06",
                        "-an", "-f", "null", "-"], capture_output=True, text=True, check=False)
    dur = re.search(r"Duration: (\d+):(\d+):([\d.]+)", p.stderr)
    total = int(dur.group(1)) * 3600 + int(dur.group(2)) * 60 + float(dur.group(3)) if dur else 0
    black = sum(float(x) for x in re.findall(r"black_duration:([\d.]+)", p.stderr))
    return total <= 0 or black >= 0.9 * total


def capture_xenia(case: dict, d: Path, base: Path) -> tuple[Path, float]:
    """Plays the route (xc.py) while recording Xenia's window; returns the
    clip and the start event's time in it."""
    x = case["xenia"]
    d.mkdir(parents=True, exist_ok=True)
    route = d / "route.txt"
    parts = []
    for r in x.get("routes", []):
        p = base / r if (base / r).exists() else HERE / "routes" / r
        parts.append(p.read_text(encoding="utf-8"))
    route.write_text("\n".join(parts), encoding="utf-8")
    game = os.environ.get("XENIA_GAME")
    if not game:
        raise CaseError("set XENIA_GAME to the extracted default.xex")
    cmd = ["uv", "run", "-q", "--no-project"]
    for w in UV_WITH:
        cmd += ["--with", w]
    cmd += ["python", "-I", str(HERE / "xc.py"), "run", str(route), "--out", str(d), "--game", game]
    if x.get("audio"):
        cmd.append("--audio")
    cmd += [f"--extra={a}" for a in x.get("args", [])]
    (d / "route.log").unlink(missing_ok=True)
    xc = subprocess.Popen(cmd)
    rec = None
    video = d / "xenia-window.mkv"
    if x["capture"] == "video":
        # Start recording once xc.py has found the window.
        while xc.poll() is None and rec is None:
            time.sleep(0.5)
            info = parse_route_log((d / "route.log").read_text(encoding="utf-8")) if (d / "route.log").exists() else {}
            if info.get("window"):
                title = window_title(info["window"])
                rec = subprocess.Popen(
                    [FFMPEG, "-hide_banner", "-y", "-f", "gdigrab", "-draw_mouse", "0", "-framerate", str(case["fps"]),
                     "-i", f"title={title}", "-c:v", "libx264", "-preset", "ultrafast", "-crf", "18", str(video)],
                    stdin=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    errs: list[str] = []
    if rec:
        t = threading.Thread(target=lambda: errs.append(rec.stderr.read()), daemon=True)
        t.start()
    xc.wait()
    if rec:
        try:
            rec.communicate("q", timeout=20)
        except (subprocess.TimeoutExpired, ValueError):
            rec.kill()
        t.join(5)
    info = parse_route_log((d / "route.log").read_text(encoding="utf-8"))
    ev = event_time(x.get("start", "t:0"), "xenia", info)
    start_epoch = ffmpeg_start_epoch("".join(errs))
    if rec and video.exists() and start_epoch and info["t0"] and not mostly_black(video):
        return video, info["t0"] + ev - start_epoch
    print("clips: no usable window recording (a disconnected session records black): using the route's shots", flush=True)
    prefix = x.get("frames", "")
    frames = [(t, d / name) for t, name in info["shots"] if name.startswith(prefix)]
    clip = d / "xenia-frames.mp4"
    first = frames_to_clip(frames, clip, int(case["fps"]))
    return clip, ev - first


def capture_ours(case: dict, d: Path, exe: Path, install: Path, tmp: Path) -> tuple[Path, float]:
    """m2-alpha offscreen, frames from dense --mission-shots (on a local
    disk, then made a clip); returns the clip and the start event's time."""
    o = case["ours"]
    d.mkdir(parents=True, exist_ok=True)
    tmp.mkdir(parents=True, exist_ok=True)
    args = ["--install", str(install), "--size", case["size"], "--screenshot", str(tmp / "m2.ppm"), *o["args"]]
    if o.get("shots"):
        args += ["--mission-shots", mission_shots(o["shots"])]
    (d / "m2-command.txt").write_text("m2-alpha " + " ".join(args), encoding="utf-8")
    env = dict(os.environ, WGPU_BACKEND="dx12")
    with open(d / "m2.log", "w") as so, open(d / "m2.err", "w") as se:
        p = subprocess.Popen([str(exe), *args], cwd=tmp, stdout=so, stderr=se, env=env,
                             creationflags=0x08000000 if os.name == "nt" else 0)
        try:
            p.wait(timeout=float(o["wall"]))
        except subprocess.TimeoutExpired:
            p.kill()
            p.wait()
    err = (d / "m2.err").read_text(encoding="utf-8", errors="replace")
    shots = parse_m2_shots(err)
    frames = [(t, Path(path)) for t, path in shots] or [(0.0, tmp / "m2.ppm")]
    clip = d / "ours-frames.mp4"
    first = frames_to_clip(frames, clip, int(case["fps"]))
    ev = event_time(o.get("start", "t:0"), "ours", m2_err=err)
    for f in tmp.glob("*.ppm"):
        f.unlink(missing_ok=True)
    return clip, ev - first


def cmd_capture(a) -> int:
    case_path = Path(a.case)
    case = load_case(case_path)
    out = Path(a.out) / case["name"]
    lock = wait_for_slot(f"side-by-side clip {case['name']}", a.wait_minutes)
    if not lock:
        print("clips: refused: the GPU slot is not ours (lock held or a GPU program running)", file=sys.stderr)
        return 3
    try:
        xclip, xstart = capture_xenia(case, out / "xenia", case_path.parent)
        oclip, ostart = capture_ours(case, out / "ours", Path(a.exe), Path(a.install), Path(a.tmp) / case["name"])
    finally:
        lock.unlink(missing_ok=True)
    meta = compose(case, oclip, xclip, ostart, xstart, out)
    print(json.dumps(meta, indent=2))
    print(out / "side-by-side.mp4")
    return 0


def cmd_compose(a) -> int:
    case = load_case(Path(a.case))
    if a.ours_offset is not None:
        case["ours"]["offset"] = a.ours_offset
    if a.xenia_offset is not None:
        case["xenia"]["offset"] = a.xenia_offset
    if a.slowmo is not None:
        case["slowmo"] = a.slowmo
    out = Path(a.out) if a.out else Path(a.ours).parent / f"{case['name']}-compose"
    ours = Path(a.ours)
    if ours.is_dir():
        frames = sorted(ours.glob("*.png")) + sorted(ours.glob("*.ppm"))
        out.mkdir(parents=True, exist_ok=True)
        clip = out / "ours-frames.mp4"
        frames_to_clip([(i / float(a.frames_fps), f) for i, f in enumerate(frames)], clip, int(case["fps"]))
        ours = clip
    meta = compose(case, ours, Path(a.xenia), a.ours_start, a.xenia_start, out)
    print(json.dumps(meta, indent=2))
    return 0


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("capture", help="capture both sides (needs the GPU slot) and compose")
    c.add_argument("case")
    c.add_argument("--exe", required=True)
    c.add_argument("--install", required=True)
    c.add_argument("--out", default=r"Z:\mercs2\scratch\clips")
    c.add_argument("--tmp", default=r"C:\Claude\scratch\clips-tmp", help="local disk for m2-alpha's frames")
    c.add_argument("--wait-minutes", type=float, default=30.0)
    m = sub.add_parser("compose", help="compose two existing clips (no GPU)")
    m.add_argument("case")
    m.add_argument("--ours", required=True, help="a clip, or a folder of frames")
    m.add_argument("--xenia", required=True)
    m.add_argument("--out")
    m.add_argument("--ours-start", type=float, default=0.0)
    m.add_argument("--xenia-start", type=float, default=0.0)
    m.add_argument("--ours-offset", type=float)
    m.add_argument("--xenia-offset", type=float)
    m.add_argument("--slowmo", type=float)
    m.add_argument("--frames-fps", type=float, default=10.0, help="rate of a --ours frames folder")
    n = sub.add_parser("notes", help="print the release-notes snippet")
    n.add_argument("case")
    k = sub.add_parser("check", help="check a case file")
    k.add_argument("case")
    a = ap.parse_args(argv)
    try:
        if a.cmd == "capture":
            return cmd_capture(a)
        if a.cmd == "compose":
            return cmd_compose(a)
        case = load_case(Path(a.case))
        if a.cmd == "notes":
            print(notes(case))
        else:
            print(f"{case['name']}: ok ({case['duration']} s at {case['fps']} fps, {case['size']} a side)")
            if case["ours"].get("shots"):
                print("m2-alpha --mission-shots", mission_shots(case["ours"]["shots"])[:80], "...")
        return 0
    except CaseError as e:
        print(f"clips: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
