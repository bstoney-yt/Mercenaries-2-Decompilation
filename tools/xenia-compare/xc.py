"""Drive Xenia Canary headless: a scripted pad, guest-frame screenshots and OCR.

Needs Windows, Python 3.12 with Pillow (and winrt-Windows.Media.Ocr for `until`),
Xenia Canary with `xinput1_4.dll` from `xpad-shim` beside its exe, started with
`--hid=xinput`. Nothing here sends anything off the machine.

    uv run --no-project --with pillow --with winrt-Windows.Media.Ocr \
        --with winrt-Windows.Graphics.Imaging --with winrt-Windows.Storage \
        --with winrt-Windows.Storage.Streams --with winrt-Windows.Foundation \
        --with winrt-Windows.Foundation.Collections --with winrt-Windows.Globalization \
        python -I xc.py run <route.txt> --out <dir> [--xenia <exe>] [--game <xex>]

Route files are one step per line (`#` comments):
    wait <s>                       sleep
    press <btn>[+<btn>] [<s>]      tap buttons (default 0.15 s), btn: a b x y lb rb
                                   start back up down left right ls rs lt rt guide
    hold <btn>[+<btn>] <s>         hold buttons
    stick <l|r> <x> <y> <s>        hold a stick (-1..1) for s seconds
    move <lx> <ly> <rx> <ry> <buttons|-> <s>   everything at once
    set <lx> <ly> <rx> <ry> <buttons|->        hold this until the next pad step
    release                        let everything go
    shot <name>                    guest-frame screenshot -> <out>/<name>.png
    burst <name> <n> <every_s>     n screenshots every_s apart (pad state kept)
    burstmove <name> <n> <every_s> <lx> <ly> <buttons|->  burst while holding
    until <text> <timeout_s> [<every_s>]   screenshot+OCR until text appears
    tap_until <btn> <text> <timeout_s>     press btn every 1.5 s until text seen
    seek <rrggbb> <timeout_s> <buttons|-> <text> [<gain>]  steer the left stick at
                                   the HUD marker of that colour until text seen
    ocr <name>                     screenshot and log its OCR text
    mark <label>                   log a timestamped label
    track <x> <y> <z>              follow the hero's position in guest memory (xmem.py)
    goto <x> <z> <radius> <timeout> <buttons|-> [<gain>]  steer there by that position
    measure <buttons|-> <secs> [<ly>]  hold, logging tracked position and speed
    trackfoot <x> <y> <z>|here     as track, for the hero on foot (here: where track was)
    walk <x> <z> <radius> <timeout> [<buttons|-> [<every_s> <name>]]  walk/sprint (ls) there
    listen <file>                  then run steps appended to <file> live, until `end`
"""

from __future__ import annotations

import argparse
import ctypes
import ctypes.wintypes as wt
import mmap
import os
import shutil
import struct
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))  # xmem.py (python -I leaves it out)

BTN = {
    "up": 0x0001, "down": 0x0002, "left": 0x0004, "right": 0x0008,
    "start": 0x0010, "back": 0x0020, "ls": 0x0040, "rs": 0x0080,
    "lb": 0x0100, "rb": 0x0200, "guide": 0x0400,
    "a": 0x1000, "b": 0x2000, "x": 0x4000, "y": 0x8000,
}

user32 = ctypes.WinDLL("user32", use_last_error=True)
WM_MENUCOMMAND = 0x0126  # Xenia's menus notify by position (MNS_NOTIFYBYPOS), ids are all 0
MF_BYPOSITION = 0x400
user32.PostMessageW.argtypes = [wt.HWND, wt.UINT, wt.WPARAM, wt.LPARAM]


class Pad:
    """The shared block `xpad-shim` reads (see its lib.rs for the layout)."""

    def __init__(self) -> None:
        self.m = mmap.mmap(-1, 16, tagname="Local\\xpad-shim")
        self.seq = 0
        self.set()

    def set(self, buttons=0, lt=0, rt=0, lx=0.0, ly=0.0, rx=0.0, ry=0.0) -> None:
        f = lambda v: max(-32767, min(32767, int(v * 32767)))
        self.seq += 1
        self.m.seek(0)
        self.m.write(struct.pack("<IHBBhhhh", self.seq, buttons, lt, rt, f(lx), f(ly), f(rx), f(ry)))


def parse_buttons(s: str):
    b, lt, rt = 0, 0, 0
    if s in ("-", ""):
        return b, lt, rt
    for name in s.lower().split("+"):
        if name == "lt":
            lt = 255
        elif name == "rt":
            rt = 255
        else:
            b |= BTN[name]
    return b, lt, rt


class Xenia:
    def __init__(self, exe: Path, shots: Path) -> None:
        self.exe, self.shots = exe, shots
        self.hwnd = None
        self.shot_id = None

    def find_window(self, pid: int, timeout=60.0) -> None:
        end = time.time() + timeout
        proto = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)
        while time.time() < end:
            found = []

            def cb(h, _):
                p = wt.DWORD()
                user32.GetWindowThreadProcessId(h, ctypes.byref(p))
                if p.value == pid and user32.GetMenu(h):
                    found.append(h)
                return True

            user32.EnumWindows(proto(cb), 0)
            if found:
                self.hwnd = found[0]
                self.shot_id = self.menu_id(user32.GetMenu(self.hwnd), "Take Screenshot")
                if self.shot_id is not None:
                    return
            time.sleep(1)
        raise RuntimeError("Xenia window or its Take Screenshot menu item not found")

    def menu_id(self, menu, want: str):
        """(menu handle, position) of the item whose text holds `want`."""
        for i in range(user32.GetMenuItemCount(menu)):
            buf = ctypes.create_unicode_buffer(256)
            user32.GetMenuStringW(menu, i, buf, 256, MF_BYPOSITION)
            sub = user32.GetSubMenu(menu, i)
            if sub:
                r = self.menu_id(sub, want)
                if r is not None:
                    return r
            elif want.lower() in buf.value.replace("&", "").lower():
                return (sub or menu, i)
        return None

    def screenshot(self, dest: Path, timeout=10.0) -> bool:
        before = set(self.shots.rglob("*.png")) if self.shots.exists() else set()
        # Xenia names screenshots to the second: keep shots a second apart.
        menu, pos = self.shot_id
        user32.PostMessageW(self.hwnd, WM_MENUCOMMAND, pos, menu)
        end = time.time() + timeout
        while time.time() < end:
            time.sleep(0.25)
            if self.shots.exists():
                new = set(self.shots.rglob("*.png")) - before
                if new:
                    f = max(new, key=lambda p: p.stat().st_mtime)
                    # Let the writer finish: wait until the size holds still.
                    size = -1
                    while f.stat().st_size != size:
                        size = f.stat().st_size
                        time.sleep(0.3)
                    dest.parent.mkdir(parents=True, exist_ok=True)
                    shutil.move(str(f), dest)
                    return True
        return False


def ocr(path: Path) -> str:
    """Windows' own OCR engine (on-device)."""
    import asyncio
    from winrt.windows.graphics.imaging import BitmapDecoder
    from winrt.windows.media.ocr import OcrEngine
    from winrt.windows.storage import FileAccessMode, StorageFile

    async def go():
        f = await StorageFile.get_file_from_path_async(str(path.resolve()))
        s = await f.open_async(FileAccessMode.READ)
        bmp = await (await BitmapDecoder.create_async(s)).get_software_bitmap_async()
        r = await OcrEngine.try_create_from_user_profile_languages().recognize_async(bmp)
        return " | ".join(l.text for l in r.lines)

    return asyncio.run(go())


def stats(path: Path) -> str:
    from PIL import Image, ImageStat

    im = Image.open(path).convert("RGB")
    st = ImageStat.Stat(im)
    return f"{im.width}x{im.height} mean={sum(st.mean) / 3:.1f} rgb={tuple(round(m) for m in st.mean)}"


def marker_x(path: Path, rgb, tol: int = 40, min_px: int = 12):
    """Mean x (0..1) of the pixels near `rgb` between 20% and 65% of the
    screen's height (above: the HUD's objective text; below: the control
    hints, whose Y button icon is the same yellow), or None if too few."""
    import numpy as np
    from PIL import Image

    a = np.asarray(Image.open(path).convert("RGB"), dtype=np.int16)
    m = (np.abs(a - np.array(rgb, dtype=np.int16)) <= tol).all(-1)
    h = a.shape[0]
    m[: h // 5] = False
    m[int(h * 0.65) :] = False
    ys, xs = np.nonzero(m)
    if len(xs) < min_px:
        return None
    return float(xs.mean()) / a.shape[1]


def run(args) -> int:
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    log = open(out / "route.log", "a", encoding="utf-8")
    t0 = time.time()

    def say(msg: str) -> None:
        line = f"{time.time() - t0:8.2f} {msg}"
        print(line, flush=True)
        log.write(line + "\n")
        log.flush()

    exe = Path(args.xenia)
    pad = Pad()
    xl = out / "xenia.log"
    cmd = [str(exe), "--gpu=d3d12", "--fullscreen=false", "--hid=xinput", f"--log_file={xl}"] + args.extra + [args.game]
    env = dict(os.environ)
    if args.audio:
        env["SDL_AUDIODRIVER"] = "disk"
        env["SDL_DISKAUDIOFILE"] = str(out / "audio.raw")
        cmd.insert(1, "--apu=sdl")
    p = subprocess.Popen(cmd, cwd=exe.parent, env=env, creationflags=0x00000080)  # HIGH_PRIORITY_CLASS
    say(f"xenia pid {p.pid}: {' '.join(cmd[1:])}")
    xe = Xenia(exe, exe.parent / "screenshots")
    xe.find_window(p.pid)
    say(f"window {xe.hwnd:#x} screenshot menu item {xe.shot_id}")
    n = 0
    held = (0, 0, 0, 0.0, 0.0, 0.0, 0.0)
    tracker = None

    def shot(name: str) -> Path | None:
        nonlocal n
        n += 1
        if p.poll() is not None:
            raise SystemExit(f"xenia exited {p.returncode}")
        d = out / f"{name}.png"
        if xe.screenshot(d):
            for _ in range(10):  # the writer may still be finishing the PNG
                try:
                    say(f"shot {d.name} {stats(d)}")
                    return d
                except OSError:
                    time.sleep(0.3)
            say(f"shot {name} unreadable")
            return None
        say(f"shot {name} FAILED")
        return None

    def apply(b=0, lt=0, rt=0, lx=0.0, ly=0.0, rx=0.0, ry=0.0):
        nonlocal held
        held = (b, lt, rt, lx, ly, rx, ry)
        pad.set(*held)

    def text_seen(text: str, tag: str) -> bool:
        d = shot(tag)
        if d is None:
            return False
        t = ocr(d)
        say(f"ocr: {t[:300]}")
        return text.lower() in t.lower()

    steps = [l.split("#", 1)[0].strip() for l in Path(args.route).read_text().splitlines()]
    try:
        def lines():
            """The route's steps; `listen <file>` then takes steps appended to
            that file (one per line) as they come, until a line `end`."""
            for st in steps:
                if not st:
                    continue
                if st.split()[0] != "listen":
                    yield st
                    continue
                f = Path(st.split(None, 1)[1])
                f.touch()
                pos = f.stat().st_size
                say(f"listening for steps in {f}")
                while p.poll() is None:
                    data = f.read_bytes()
                    if len(data) > pos:
                        new, pos = data[pos:].decode("utf-8", "replace"), len(data)
                        for cmd in (c.split("#", 1)[0].strip() for c in new.splitlines()):
                            if cmd == "end":
                                return
                            if cmd:
                                say(f"step: {cmd}")
                                yield cmd
                    time.sleep(0.3)

        for i, line in enumerate(lines()):
            w = line.split()
            op = w[0]
            if op == "wait":
                time.sleep(float(w[1]))
            elif op in ("press", "hold"):
                b, lt, rt = parse_buttons(w[1])
                dur = float(w[2]) if len(w) > 2 else 0.15
                kept = held
                apply(b | kept[0], max(lt, kept[1]), max(rt, kept[2]), *kept[3:])
                time.sleep(dur)
                apply(*kept)
                time.sleep(0.1)
            elif op == "stick":
                x, y, dur = float(w[2]), float(w[3]), float(w[4])
                apply(*held[:3], *((x, y, 0, 0) if w[1] == "l" else (0, 0, x, y)))
                time.sleep(dur)
                apply()
            elif op == "move":
                b, lt, rt = parse_buttons(w[5])
                apply(b, lt, rt, float(w[1]), float(w[2]), float(w[3]), float(w[4]))
                time.sleep(float(w[6]))
                apply()
            elif op == "set":
                b, lt, rt = parse_buttons(w[5])
                apply(b, lt, rt, float(w[1]), float(w[2]), float(w[3]), float(w[4]))
            elif op == "release":
                apply()
            elif op == "shot":
                shot(w[1])
            elif op in ("burst", "burstmove"):
                k, every = int(w[2]), float(w[3])
                if op == "burstmove":
                    b, lt, rt = parse_buttons(w[6])
                    apply(b, lt, rt, float(w[4]), float(w[5]), 0, 0)
                for j in range(k):
                    t = time.time()
                    shot(f"{w[1]}-{j:03d}")
                    time.sleep(max(0.0, every - (time.time() - t)))
                if op == "burstmove":
                    apply()
            elif op == "until":
                text, timeout = w[1].replace("_", " "), float(w[2])
                every = float(w[3]) if len(w) > 3 else 3.0
                end = time.time() + timeout
                while not text_seen(text, f"until-{i:03d}"):
                    if time.time() > end:
                        say(f"until '{text}' TIMED OUT")
                        break
                    time.sleep(every)
                else:
                    say(f"until '{text}' seen")
            elif op == "tap_until":
                b, lt, rt = parse_buttons(w[1])
                text, timeout = w[2].replace("_", " "), float(w[3])
                end = time.time() + timeout
                while not text_seen(text, f"tap-{i:03d}"):
                    if time.time() > end:
                        say(f"tap_until '{text}' TIMED OUT")
                        break
                    apply(b, lt, rt)
                    time.sleep(0.15)
                    apply()
                    time.sleep(1.5)
            elif op == "ocr":
                d = shot(w[1])
                if d:
                    say(f"ocr {w[1]}: {ocr(d)[:400]}")
            elif op == "seek":
                # seek <rrggbb> <timeout_s> <buttons|-> <until_text> [<gain>]
                # Holds the buttons and steers the left stick so the HUD marker
                # of that colour sits mid-screen, until OCR sees the text. When
                # the marker stays well off-centre (circling it), it lets go of
                # the buttons and brakes (LT) for a moment to tighten the turn.
                rgb = tuple(int(w[1][k:k + 2], 16) for k in (0, 2, 4))
                timeout, (b, lt, rt) = float(w[2]), parse_buttons(w[3])
                text = w[4].replace("_", " ")
                gain = float(w[5]) if len(w) > 5 else 0.8
                end, lx, j, off = time.time() + timeout, 0.0, 0, 0
                while time.time() < end:
                    apply(b, lt, rt, lx, 0.0, 0.0, 0.0)
                    d = shot(f"seek-{i:03d}-{j:03d}")
                    j += 1
                    if d is None:
                        continue
                    cx = marker_x(d, rgb)
                    t = ocr(d)
                    if text.lower() in t.lower():
                        say(f"seek: '{text}' seen")
                        break
                    if cx is None:
                        say(f"seek: no marker; ocr: {t[:120]}")
                        lx, off = 0.0, 0  # straight on: the marker shows within 175 m
                    else:
                        lx = max(-0.7, min(0.7, gain * (cx - 0.5) * 2))
                        off = off + 1 if abs(cx - 0.5) > 0.15 else 0
                        say(f"seek: marker at x={cx:.2f}, stick {lx:+.2f}, off-centre {off}")
                        if off >= 4:
                            apply(0, 255, 0, 1.0 if cx > 0.5 else -1.0, 0.0, 0.0, 0.0)
                            time.sleep(1.5)
                            off = 0
                    time.sleep(0.9)
                else:
                    say(f"seek '{text}' TIMED OUT")
                apply()
            elif op == "track":
                # track <x> <y> <z>: find the vehicle's world matrix near there
                # in guest memory (the one that moves forward on A) and follow
                # it (xmem.MatrixTracker), for goto.
                import xmem

                g = xmem.Guest(p.pid)

                def throttle():
                    apply(BTN["a"])
                    time.sleep(1.2)
                    apply()

                tracker, n_m, moved = xmem.MatrixTracker.find(g, tuple(float(v) for v in w[1:4]), throttle)
                if tracker is None:
                    say(f"track: no moving world matrix among {n_m}; falling back to the buffer tracker")
                    tracker = xmem.Tracker(g, tuple(float(v) for v in w[1:4]))
                else:
                    say(f"track: {len(tracker.addrs)} moving world matrices of {n_m} (first {tracker.addr:#x}, {moved:.1f} m forward); at {tracker.update()}")
            elif op == "measure":
                # measure <buttons|-> <secs> [<ly>]: hold the buttons (and the
                # left stick at ly) and log the tracked position and speed
                # every 0.25 s (for braking, coasting, sprint speeds).
                import math

                b, lt, rt = parse_buttons(w[1])
                secs = float(w[2])
                ly = float(w[3]) if len(w) > 3 else 0.0
                apply(b, lt, rt, 0.0, ly, 0.0, 0.0)
                t0, last = time.time(), None
                while time.time() - t0 < secs:
                    pos = tracker.update() if tracker else None
                    now = time.time()
                    if pos is not None:
                        v = None if last is None else math.hypot(pos[0] - last[1][0], pos[2] - last[1][2]) / (now - last[0])
                        say(f"measure {w[1]} t={now - t0:5.2f} pos=({pos[0]:.1f}, {pos[2]:.1f}) v={'-' if v is None else f'{v:.2f}'} m/s")
                        last = (now, pos)
                    time.sleep(0.25)
                apply()
            elif op == "trackfoot":
                # trackfoot <x> <y> <z>: as track, for the hero on foot (found
                # by walking forward for a moment instead of the throttle).
                import xmem

                g = xmem.Guest(p.pid)

                def forward():
                    apply(0, 0, 0, 0.0, 1.0, 0.0, 0.0)
                    time.sleep(1.5)
                    apply()

                start = tracker.pos if w[1] == "here" and tracker else tuple(float(v) for v in w[1:4])
                tracker, n_m, moved = xmem.MatrixTracker.find(g, start, forward)
                if tracker is None:
                    say(f"trackfoot: no moving world matrix among {n_m}")
                else:
                    say(f"trackfoot: {len(tracker.addrs)} moving matrices of {n_m}, {moved:.1f} m forward; at {tracker.update()}")
            elif op == "walk":
                # walk <x> <z> <radius_m> <timeout_s> [<buttons|->] [<shots_every_s> <name>]:
                # on foot the stick is camera-relative, so the camera's yaw is
                # estimated from where she goes against where the stick points
                # (stick angle a to the right of the camera moves her on
                # bearing yaw - a in this world), and the stick aimed at the
                # point from it. With ls held she sprints. Frames on the way
                # with shots_every_s.
                import math

                tx, tz, radius, timeout = (float(v) for v in w[1:5])
                b, lt, rt = parse_buttons(w[5]) if len(w) > 5 else (0, 0, 0)
                every = float(w[6]) if len(w) > 6 else 0.0
                name = w[7] if len(w) > 7 else f"walk{i:03d}"
                end, yaw, a, k, next_shot = time.time() + timeout, None, 0.0, 0, time.time()
                while time.time() < end:
                    pos = tracker.update() if tracker else None
                    if pos is None:
                        time.sleep(0.1)
                        continue
                    dist = math.hypot(tx - pos[0], tz - pos[2])
                    if dist <= radius:
                        say(f"walk: at ({pos[0]:.0f}, {pos[2]:.0f}), {dist:.0f} m from the point")
                        break
                    want = math.degrees(math.atan2(tx - pos[0], tz - pos[2]))
                    travel = tracker.heading() if len(tracker.trail) >= 2 else None
                    if travel is not None:
                        est = (travel + a + 180) % 360 - 180
                        yaw = est if yaw is None else yaw + 0.5 * ((est - yaw + 180) % 360 - 180)
                    a = 0.0 if yaw is None else (yaw - want + 180) % 360 - 180
                    apply(b, lt, rt, math.sin(math.radians(a)), math.cos(math.radians(a)), 0.0, 0.0)
                    if every and time.time() >= next_shot:
                        next_shot = time.time() + every
                        shot(f"{name}-{k:03d}")
                        k += 1
                    elif k % 5 == 0:
                        say(f"walk: ({pos[0]:.0f}, {pos[2]:.0f}) want {want:.0f} yaw {yaw if yaw is None else round(yaw)} stick {a:+.0f} deg dist {dist:.0f}")
                    time.sleep(0.15)
                else:
                    say(f"walk ({tx:.0f}, {tz:.0f}) TIMED OUT")
                apply()
            elif op == "goto":
                # goto <x> <z> <radius_m> <timeout_s> <buttons|-> [<gain_deg>]:
                # hold the buttons (the boat's throttle is A) and steer the left
                # stick at the point by the tracked heading, until within radius.
                # Measured: stick right turns the boat from +z towards -x (the
                # bearing atan2(dx, dz) goes down), so the stick is -error.
                import math

                tx, tz, radius, timeout = (float(v) for v in w[1:5])
                b, lt, rt = parse_buttons(w[5])
                gain = float(w[6]) if len(w) > 6 else 60.0
                end, last_log, misses, lx = time.time() + timeout, 0.0, 0, 0.0
                # Stalled (under 3 m in 4 s, say aground): back off with X (the
                # boat's brake/reverse) and the stick the other way for 2 s.
                stall_t, stall_p = time.time(), None
                while time.time() < end:
                    pos = tracker.update()
                    if pos is None:
                        misses += 1
                        if misses > 10:
                            say("goto: lost the track")
                            break
                        time.sleep(0.1)
                        continue
                    misses = 0
                    if stall_p is None:
                        stall_p = pos
                    if time.time() - stall_t > 4.0:
                        if math.hypot(pos[0] - stall_p[0], pos[2] - stall_p[2]) < 3.0:
                            say(f"goto: stalled at ({pos[0]:.0f}, {pos[2]:.0f}), backing off")
                            apply(BTN["x"], 0, 0, -lx if lx else 1.0, 0.0, 0.0, 0.0)
                            time.sleep(2.0)
                            tracker.update()
                        stall_t, stall_p = time.time(), None
                    dist = math.hypot(tx - pos[0], tz - pos[2])
                    if dist <= radius:
                        say(f"goto: at ({pos[0]:.0f}, {pos[2]:.0f}), {dist:.0f} m from the point")
                        break
                    hdg = tracker.heading()
                    want = math.degrees(math.atan2(tx - pos[0], tz - pos[2]))
                    # Well off course: let the throttle go and turn on full lock
                    # (at speed the turning circle is ~100 m and full lock spins
                    # the boat); lined up: throttle with gentle corrections.
                    gas = True
                    if hdg is not None:
                        err = (want - hdg + 180) % 360 - 180
                        gas = abs(err) < 45
                        cap = 0.6 if gas else 1.0
                        lx = max(-cap, min(cap, -err / gain))
                    apply(b if gas else 0, lt if gas else 0, rt if gas else 0, lx, 0.0, 0.0, 0.0)
                    if time.time() - last_log > 1.0:
                        last_log = time.time()
                        say(f"goto: ({pos[0]:.0f}, {pos[2]:.0f}) heading {hdg if hdg is None else round(hdg)} want {want:.0f} dist {dist:.0f} stick {lx:+.2f}")
                    time.sleep(0.15)
                else:
                    say(f"goto ({tx:.0f}, {tz:.0f}) TIMED OUT")
                apply()
            elif op == "mark":
                say("mark " + " ".join(w[1:]))
            else:
                raise ValueError(f"unknown step: {line}")
            if p.poll() is not None:
                say(f"xenia exited {p.returncode}")
                return 1
        say("route done")
    finally:
        if not args.keep:
            p.kill()
            say("xenia closed")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("route")
    r.add_argument("--out", required=True)
    r.add_argument("--xenia", default=os.environ.get("XENIA_EXE", r"C:\Claude\tools\xenia\canary\m2-alpha-xenia.exe"))
    r.add_argument("--game", default=os.environ.get("XENIA_GAME", ""))
    r.add_argument("--audio", action="store_true", help="dump SDL audio to <out>/audio.raw")
    r.add_argument("--keep", action="store_true", help="leave Xenia running after the route")
    r.add_argument("--extra", action="append", default=[], help="one Xenia argument (repeat; use --extra=--x=y)")
    o = sub.add_parser("ocr")
    o.add_argument("png")
    a = ap.parse_args()
    if a.cmd == "ocr":
        print(ocr(Path(a.png)))
        return 0
    if not a.game:
        sys.exit("--game or XENIA_GAME is required")
    return run(a)


if __name__ == "__main__":
    sys.exit(main())
