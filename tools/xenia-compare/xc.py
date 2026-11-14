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
    if_text <text> <step...>       run the step only if the screen's OCR holds the text
    mark <label>                   log a timestamped label
    track <x> <y> <z>              follow the hero's position in guest memory (xmem.py)
    goto <x> <z> <radius> <timeout> <buttons|-> [<gain>] [max=<m/s>]  steer there by that position
    measure <buttons|-> <secs> [<ly>]  hold, logging tracked position and speed
    walkcam <x> <z> <radius> <timeout> [<buttons|-> [<every_s> <name>]]  stick forward, steer by the camera
    airlog start <x> <z> <name> | airlog stop   log state-table movers near (x, z) in the background
    stickresp [moving]             measure the camera's response to the right stick
    fight2 <secs> [<every_s> <name>] approach, aim (yaw and pitch) and fire at ground movers
    fight <secs> [<every_s> <name>]  aim at the nearest slow ground mover and fire in bursts
    airwatch <dropx> <dropz> <secs> [<every_s> <name>]  log movers near a drop, camera on the fastest airborne one
    look <bearing> [<timeout>]     turn the camera to face that bearing (xmem.Camera)
    trackhero                      follow the player's character by the 360 code's tables (xmem.Hero)
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


def cam_stick(err: float, fine: float = 3.0) -> float:
    """Right-stick deflection to close a camera error (degrees), from the
    measured on-foot response (round 12, stickresp; standing and running
    alike): |0.25| does nothing, 0.5 ~0.6 deg/s, 0.75 ~17 deg/s (pitch
    14-20), 1.0 ~170+ deg/s yaw (pitch 44-58): a dead zone to about 0.5 and a
    steep curve, so it is stepped, not proportional. The sign is the error's;
    callers flip it for yaw (+x lowers the bearing)."""
    a = abs(err)
    if a > 30:
        mag = 1.0
    elif a > fine:
        mag = 0.75
    else:
        return 0.0
    return mag if err > 0 else -mag


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
    airlog_state: dict = {}

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

        pending: list[str] = []

        def all_lines():
            for st in lines():
                yield st
                while pending:
                    yield pending.pop(0)

        for i, line in enumerate(all_lines()):
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
                m_t0, last = time.time(), None
                while time.time() - m_t0 < secs:
                    pos = tracker.update() if tracker else None
                    now = time.time()
                    if pos is not None:
                        v = None if last is None else math.hypot(pos[0] - last[1][0], pos[2] - last[1][2]) / (now - last[0])
                        say(f"measure {w[1]} t={now - m_t0:5.2f} pos=({pos[0]:.1f}, {pos[2]:.1f}) v={'-' if v is None else f'{v:.2f}'} m/s")
                        last = (now, pos)
                    time.sleep(0.25)
                apply()
            elif op == "walkcam":
                # walkcam <x> <z> <radius> <timeout> [<buttons|->] [<every_s> <name>]:
                # left stick held forward; steered by turning the camera with
                # the right stick. Her measured travel (exact position from
                # xmem.Hero) against the camera's yaw (xmem.Camera) gives the
                # offset between them; the camera is turned to want - offset.
                # The right stick's sense is learnt from the camera's response.
                # ls is clicked once, a second in (sprint, tControlMap).
                import math
                import xmem

                tx, tz, radius, timeout = (float(v) for v in w[1:5])
                b, lt, rt = parse_buttons(w[5]) if len(w) > 5 else (0, 0, 0)
                every = float(w[6]) if len(w) > 6 else 0.0
                name = w[7] if len(w) > 7 else f"walkcam{i:03d}"
                cam = xmem.Camera(xmem.Guest(p.pid))
                end, wc_t0, k, next_shot, last_say = time.time() + timeout, time.time(), 0, time.time(), 0.0
                offset, sense, rx = 0.0, 1.0, 0.0
                last_yaw, clicked = None, 0.0
                stall_t, stall_p, jump_t = time.time(), None, 0.0
                while time.time() < end:
                    pos = tracker.update() if tracker else None
                    yv = cam.yaw()
                    if pos is None or yv is None:
                        time.sleep(0.1)
                        continue
                    dist = math.hypot(tx - pos[0], tz - pos[2])
                    if dist <= radius:
                        say(f"walkcam: at ({pos[0]:.0f}, {pos[2]:.0f}), {dist:.0f} m from the point")
                        break
                    want = math.degrees(math.atan2(tx - pos[0], tz - pos[2]))
                    trail = getattr(tracker, "trail", [])
                    if len(trail) >= 2:
                        (t1, p1), (t2, p2) = trail[0], trail[-1]
                        if t2 - t1 > 0.3 and math.hypot(p2[0] - p1[0], p2[2] - p1[2]) / (t2 - t1) > 0.5:
                            travel = math.degrees(math.atan2(p2[0] - p1[0], p2[2] - p1[2]))
                            d = (travel - yv - offset + 180) % 360 - 180
                            offset = (offset + 0.2 * d + 180) % 360 - 180
                    # learn the right stick's sense: pushed, did the yaw move the way meant?
                    if last_yaw is not None and abs(rx) > 0.3:
                        moved = (yv - last_yaw + 180) % 360 - 180
                        if abs(moved) > 1.0 and (moved > 0) != (sense * rx > 0):
                            sense = -sense
                    last_yaw = yv
                    err = (want - offset - yv + 180) % 360 - 180
                    rx = -cam_stick(err, fine=6.0)  # measured: +x lowers the bearing
                    bb = b & ~BTN["ls"]
                    # Stalled (under 1 m in 2 s: a bank or ledge at the water's
                    # edge, z ~2640 on VzaCon001's beach): jump (A) while pushing on.
                    if stall_p is None or time.time() - stall_t > 2.0:
                        if stall_p is not None and math.hypot(pos[0] - stall_p[0], pos[2] - stall_p[2]) < 1.0:
                            jump_t = time.time()
                            say(f"walkcam: stalled at ({pos[0]:.0f}, {pos[2]:.0f}), jumping")
                        stall_t, stall_p = time.time(), pos
                    if time.time() - jump_t < 0.3:
                        bb |= BTN["a"]
                    if b & BTN["ls"] and not clicked and time.time() - wc_t0 > 1.0:
                        clicked = time.time()
                    if clicked and time.time() - clicked < 0.2:
                        bb |= BTN["ls"]
                    apply(bb, lt, rt, 0.0, 1.0, rx, 0.0)
                    if every and time.time() >= next_shot:
                        next_shot = time.time() + every
                        shot(f"{name}-{k:03d}")
                        k += 1
                    if time.time() - last_say > 1.0:
                        last_say = time.time()
                        say(f"walkcam: ({pos[0]:.0f}, {pos[2]:.0f}) want {want:.0f} cam {yv:.0f} offset {offset:+.0f} rx {rx:+.2f} dist {dist:.0f}")
                    time.sleep(0.1)
                else:
                    say(f"walkcam ({tx:.0f}, {tz:.0f}) TIMED OUT")
                apply()
            elif op == "airlog":
                # airlog start <x> <z> <name> | airlog stop: in the background,
                # every 0.25 s, log every state-table entry moving over 3 m/s
                # within 1200 m of (x, z) to <out>/<name>.csv, timed on the
                # route's clock (the t of the say() lines), until stopped.
                import csv
                import math
                import threading
                import xmem

                if w[1] == "stop":
                    if airlog_state.get("stop"):
                        airlog_state["stop"].set()
                        airlog_state["thread"].join(5)
                        say("airlog: stopped")
                else:
                    ax, az, name = float(w[2]), float(w[3]), w[4]
                    stop_ev = threading.Event()

                    def logger(ax=ax, az=az, name=name, stop_ev=stop_ev):
                        g2 = xmem.Guest(p.pid)
                        with open(out / f"{name}.csv", "w", newline="") as fh:
                            wr = csv.writer(fh)
                            wr.writerow(["t", "index", "x", "y", "z", "speed"])
                            last, prev = {}, None
                            while not stop_ev.is_set():
                                now = time.time()
                                ents = xmem.state_entries(g2)
                                if prev is not None:
                                    dt = now - prev
                                    for idx, pt in ents.items():
                                        q = last.get(idx)
                                        if q is None or math.hypot(pt[0] - ax, pt[2] - az) > 1200:
                                            continue
                                        v = math.dist(pt, q) / dt
                                        if 3.0 < v < 150.0:
                                            wr.writerow([f"{now - t0:.2f}", idx, f"{pt[0]:.1f}", f"{pt[1]:.1f}", f"{pt[2]:.1f}", f"{v:.1f}"])
                                last, prev = ents, now
                                stop_ev.wait(0.25)

                    th = threading.Thread(target=logger, daemon=True)
                    th.start()
                    airlog_state.update(stop=stop_ev, thread=th)
                    say(f"airlog: logging movers within 1200 m of ({ax:.0f}, {az:.0f}) to {name}.csv")
            elif op == "fight":
                # fight <secs> [<every_s> <name>]: fire RT (tControlMap: RT fire on
                # foot; LT is crouch) at the nearest slow ground mover within 60 m (a soldier:
                # moved 1-8 m/s over the last second, near the hero's height),
                # turning the camera with the right stick towards it; OCR logs
                # the HUD (the "Kill enemies (n/4)" count) every few seconds.
                import math
                import xmem

                secs = float(w[1])
                every = float(w[2]) if len(w) > 2 else 2.0
                name = w[3] if len(w) > 3 else f"fight{i:03d}"
                g = xmem.Guest(p.pid)
                cam = xmem.Camera(g)
                hist: dict[int, tuple] = {}
                end, k, next_shot, sense, rx, last_yaw = time.time() + secs, 0, time.time(), 1.0, 0.0, None
                while time.time() < end:
                    now = time.time()
                    hp = tracker.update() if tracker else None
                    ents = xmem.state_entries(g)
                    target = None
                    if hp is not None:
                        cands = []
                        for idx, pt in ents.items():
                            old = hist.get(idx)
                            if old and now - old[0] > 0.8:
                                d_hero = math.hypot(pt[0] - hp[0], pt[2] - hp[2])
                                v = math.dist(pt, old[1]) / (now - old[0])
                                if 1.0 < v < 8.0 and 3.0 < d_hero < 150.0 and abs(pt[1] - hp[1]) < 15:
                                    cands.append((d_hero, pt))
                        best = min(cands) if cands else None
                        target, t_dist = (best[1], best[0]) if best else (None, 0.0)
                    for idx, pt in ents.items():
                        if idx not in hist or now - hist[idx][0] > 1.0:
                            hist[idx] = (now, pt)
                    fire, advance = 0, 0.0
                    if target is not None:
                        bearing = math.degrees(math.atan2(target[0] - hp[0], target[2] - hp[2]))
                        yv = cam.yaw()
                        if yv is not None:
                            if last_yaw is not None and abs(rx) > 0.3:
                                moved = (yv - last_yaw + 180) % 360 - 180
                                if abs(moved) > 1.0 and (moved > 0) != (sense * rx > 0):
                                    sense = -sense
                            last_yaw = yv
                            err = (bearing - yv + 180) % 360 - 180
                            rx = max(-1.0, min(1.0, sense * err / 40))
                            # over 35 m: run at it (the camera faces it, so stick
                            # forward); within 35 m: stand and fire when on it.
                            advance = 1.0 if t_dist > 35.0 and abs(err) < 30 else 0.0
                            fire = 255 if abs(err) < 12 and t_dist <= 35.0 else 0
                    else:
                        rx = 0.0
                    apply(0, 0, fire, 0.0, advance, rx, 0.0)
                    if now >= next_shot:
                        next_shot = now + every
                        d = shot(f"{name}-{k:03d}")
                        k += 1
                        if d is not None:
                            t_ocr = ocr(d)
                            say(f"fight: target {target is not None}; ocr: {t_ocr[:80]}")
                            if "completed" in t_ocr.lower() and "kill" in t_ocr.lower():
                                say("fight: Kill enemies completed")
                                break
                    time.sleep(0.05)
                apply()
            elif op == "fight2":
                # fight2 <secs> [<every_s> <name>]: clear "Kill enemies". Target: the
                # nearest ground mover within 150 m (state table), kept until it
                # has not moved for 5 s (counted as down). Over 25 m: stick
                # forward, the camera steered at it (as walkcam); within 25 m:
                # stand, pitch the camera (right stick y) at its chest, and fire
                # RT in bursts when within 8 deg across and 6 deg up/down.
                # Logs distance and errors; OCR of the HUD every few seconds.
                import math
                import xmem

                secs = float(w[1])
                every = float(w[2]) if len(w) > 2 else 3.0
                name = w[3] if len(w) > 3 else f"fight{i:03d}"
                # at x,y,z;x,y,z...: fixed targets (the mission layer's own
                # placements, e.g. VzaCon001's vza_beachguard_1..4, which stand
                # still), cycled every 10 s, instead of movers from the table.
                fixed = []
                if len(w) > 5 and w[4] == "at":
                    fixed = [tuple(float(v) for v in t.split(",")) for t in w[5].split(";")]
                fixed_i, fixed_t = 0, time.time()
                stand_off = next((float(t[9:]) for t in w if t.startswith("standoff=")), 25.0)
                g = xmem.Guest(p.pid)
                cam = xmem.Camera(g)
                seen: dict[int, tuple] = {}   # index -> (t last moved, pos)
                downed: set[int] = set()
                tgt = None
                sx, sy, rx, ry, last_yaw, last_pitch = 1.0, 1.0, 0.0, 0.0, None, None
                offset, end, k, next_shot, last_say = 0.0, time.time() + secs, 0, time.time(), 0.0
                while time.time() < end:
                    now = time.time()
                    hp = tracker.update() if tracker else None
                    ents = xmem.state_entries(g)
                    for idx, pt in ents.items():
                        old = seen.get(idx)
                        if old is None or math.dist(pt, old[1]) > 0.3:
                            seen[idx] = (now, pt)
                    if hp is None:
                        time.sleep(0.1)
                        continue
                    if not fixed and tgt is not None and now - seen.get(tgt, (0, None))[0] > 5.0:
                        downed.add(tgt)
                        say(f"fight2: target {tgt} still for 5 s: counted down ({len(downed)})")
                        tgt = None
                    if tgt is None and not fixed:
                        cands = []
                        for idx, (tm, pt) in seen.items():
                            if idx in downed or now - tm > 1.5 or idx not in ents:
                                continue
                            d = math.hypot(pt[0] - hp[0], pt[2] - hp[2])
                            # the soldiers gather near (-1466, 2772) (r10 state table)
                            if 3.0 < d < 150.0 and abs(pt[1] - hp[1]) < 15 and math.hypot(pt[0] + 1466, pt[2] - 2772) < 60:
                                cands.append((d, idx))
                        tgt = min(cands)[1] if cands else None
                    fire, ly = 0, 0.0
                    if fixed:
                        if now - fixed_t > 10.0:
                            fixed_i, fixed_t = (fixed_i + 1) % len(fixed), now
                        tgt = f"guard{fixed_i + 1}"
                        tp = fixed[fixed_i]
                    elif tgt is not None and (tgt not in ents or math.hypot(ents[tgt][0] - hp[0], ents[tgt][2] - hp[2]) > 150):
                        tgt = None  # gone, or the slot reused for something far away
                    if tgt is not None and not fixed:
                        tp = ents[tgt]
                    if tgt is not None:
                        d = math.hypot(tp[0] - hp[0], tp[2] - hp[2])
                        bearing = math.degrees(math.atan2(tp[0] - hp[0], tp[2] - hp[2]))
                        yv, pv, cp = cam.yaw(), cam.pitch(), cam.pos()
                        if yv is not None:
                            # Measured (look, r6d): right stick left (-) raises the
                            # camera's yaw; fixed, not learnt (learning flip-flopped).
                            err = (bearing - yv + 180) % 360 - 180
                            rx = -cam_stick(err)
                            perr = 0.0
                            if pv is not None and cp is not None:
                                hd = max(1.0, math.hypot(tp[0] - cp[0], tp[2] - cp[2]))
                                want_p = math.degrees(math.atan2(tp[1] + 1.2 - cp[1], hd))
                                perr = want_p - pv
                                ry = cam_stick(perr)  # measured: +y raises the pitch
                            if d > stand_off:
                                ly = 1.0
                            elif abs(err) < 4 and abs(perr) < 4 and int(now * 2.5) % 2 == 0:
                                fire = 255
                            if now - last_say > 1.0:
                                last_say = now
                                say(f"fight2: target {tgt} at {d:.0f} m, yaw err {err:+.0f}, pitch err {perr:+.0f}, {'advance' if ly else ('FIRE' if fire else 'aim')}, down {len(downed)}")
                    else:
                        rx = ry = 0.0
                    apply(0, 0, fire, 0.0, ly, rx, ry)
                    if now >= next_shot:
                        next_shot = now + every
                        dd = shot(f"{name}-{k:03d}")
                        k += 1
                        if dd is not None:
                            t_ocr = ocr(dd)
                            if t_ocr:
                                say(f"fight2: ocr: {t_ocr[:90]}")
                            low = t_ocr.lower()
                            if "completed" in low and "kill" in low:
                                say("fight2: Kill enemies completed")
                                break
                            if "medevac" in low or "stock up" in low or "loading" in low:
                                say("fight2: a loading tip: she is down, stopping")
                                break
                    time.sleep(0.05)
                apply()
            elif op == "stickresp":
                # stickresp [moving]: hold the right stick at x then y = +-0.25,
                # 0.5, 0.75, 1.0 for 1.5 s each (1 s rest between), sampling the
                # camera's yaw, pitch and position (xmem.Camera) every 0.1 s;
                # logs each hold's mean rate (deg/s) over its last second.
                # "moving": the left stick held forward throughout.
                import xmem

                cam = xmem.Camera(xmem.Guest(p.pid))
                ly = 1.0 if len(w) > 1 and w[1] == "moving" else 0.0
                for axis in ("x", "y"):
                    for v in (0.25, 0.5, 0.75, 1.0, -0.25, -0.5, -0.75, -1.0):
                        apply(0, 0, 0, 0.0, ly, v if axis == "x" else 0.0, v if axis == "y" else 0.0)
                        t_s, samples = time.time(), []
                        while time.time() - t_s < 1.5:
                            samples.append((time.time(), cam.yaw(), cam.pitch()))
                            time.sleep(0.1)
                        apply(0, 0, 0, 0.0, ly, 0.0, 0.0)
                        late = [x for x in samples if x[0] - t_s >= 0.5 and x[1] is not None]
                        if len(late) >= 2:
                            (ta, ya, pa), (tb, yb, pb) = late[0], late[-1]
                            dy = (yb - ya + 180) % 360 - 180
                            say(f"stickresp {'moving' if ly else 'standing'} r{axis}={v:+.2f}: yaw {dy / (tb - ta):+7.1f} deg/s, pitch {(pb - pa) / (tb - ta):+6.1f} deg/s (pitch now {pb:+.1f})")
                        time.sleep(1.0)
                apply()
            elif op == "airwatch":
                # airwatch <dropx> <dropz> <secs> [<every_s> <name>]: every 0.25 s
                # read the whole state table (xmem.state_entries) and log every
                # entry moving over 3 m/s within 800 m of the drop point to
                # <out>/<name>.csv (t, index, x, y, z, speed); turn the camera
                # with the right stick at the fastest one at least 5 m up from
                # the hero (the delivery helicopter), frames every every_s.
                import csv
                import math
                import xmem

                dx, dz, secs = float(w[1]), float(w[2]), float(w[3])
                every = float(w[4]) if len(w) > 4 else 1.0
                name = w[5] if len(w) > 5 else f"air{i:03d}"
                g = xmem.Guest(p.pid)
                cam = xmem.Camera(g)
                f = open(out / f"{name}.csv", "w", newline="")
                wr = csv.writer(f)
                wr.writerow(["t", "index", "x", "y", "z", "speed"])
                aw_t0, last, prev_t, k, next_shot, sense, rx, last_yaw = time.time(), {}, None, 0, time.time(), 1.0, 0.0, None
                hero_y = tracker.pos[1] if tracker and getattr(tracker, "pos", None) else -35.0
                best_i = None
                while time.time() - aw_t0 < secs:
                    now = time.time()
                    ents = xmem.state_entries(g)
                    hp = tracker.update() if tracker else None
                    movers = []
                    if prev_t is not None:
                        dt = now - prev_t
                        for idx, pt in ents.items():
                            q = last.get(idx)
                            if q is None or math.hypot(pt[0] - dx, pt[2] - dz) > 800:
                                continue
                            v = math.dist(pt, q) / dt
                            if 3.0 < v < 120.0:
                                movers.append((idx, pt, v))
                                wr.writerow([f"{now - aw_t0:.2f}", idx, f"{pt[0]:.1f}", f"{pt[1]:.1f}", f"{pt[2]:.1f}", f"{v:.1f}"])
                    last, prev_t = ents, now
                    air = [m for m in movers if m[1][1] > hero_y + 5 and (hp is None or math.hypot(m[1][0] - hp[0], m[1][2] - hp[2]) > 3)]
                    if air:
                        best_i, bp, bv = max(air, key=lambda m: m[2])
                        if hp is not None:
                            bearing = math.degrees(math.atan2(bp[0] - hp[0], bp[2] - hp[2]))
                            yv = cam.yaw()
                            if yv is not None:
                                if last_yaw is not None and abs(rx) > 0.3:
                                    moved = (yv - last_yaw + 180) % 360 - 180
                                    if abs(moved) > 1.0 and (moved > 0) != (sense * rx > 0):
                                        sense = -sense
                                last_yaw = yv
                                err = (bearing - yv + 180) % 360 - 180
                                rx = max(-1.0, min(1.0, sense * err / 40))
                    else:
                        rx = 0.0
                    apply(0, 0, 0, 0.0, 0.0, rx, 0.0)
                    if now >= next_shot:
                        next_shot = now + every
                        shot(f"{name}-{k:03d}")
                        k += 1
                        say(f"airwatch: {len(movers)} moving, airborne {len(air)}; following {best_i}")
                    time.sleep(0.05)
                f.close()
                apply()
            elif op == "look":
                # look <bearing> [<timeout>]: turn the camera with the right stick
                # until it faces that bearing (degrees from +z towards +x), the
                # stick's sense learnt from the first push.
                import xmem

                cam = xmem.Camera(xmem.Guest(p.pid))
                want, timeout = float(w[1]), float(w[2]) if len(w) > 2 else 8.0
                end, sense = time.time() + timeout, 1.0
                last_err = None
                while time.time() < end:
                    yv = cam.yaw()
                    if yv is None:
                        break
                    err = (want - yv + 180) % 360 - 180
                    if abs(err) < 5:
                        break
                    if last_err is not None and abs(err) > abs(last_err) + 1:
                        sense = -sense
                    last_err = err
                    apply(*held[:3], held[3], held[4], max(-1.0, min(1.0, sense * -err / 45)), 0.0)
                    time.sleep(0.15)
                apply(*held[:3], held[3], held[4], 0.0, 0.0)
                say(f"look: camera yaw {cam.yaw()} (wanted {want:.0f})")
            elif op == "trackhero":
                # trackhero: follow the local player's character through the
                # 360 code's own tables (xmem.Hero): one record, in a vehicle
                # or on foot, position and facing.
                import xmem

                tracker = xmem.Hero(xmem.Guest(p.pid))
                for _ in range(30):  # the player may not be registered yet
                    if tracker.update() is not None:
                        break
                    time.sleep(0.5)
                say(f"trackhero: character {tracker.character()} at {tracker.update()}, facing {tracker.facing()}")
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
                found = None
                for tol in (15.0, 40.0, 40.0):  # out of a boat she can be a few metres off
                    # The hero's matrices were found over the whole range (run 16),
                    # not only in the boat's 0xA0000000 view.
                    found, n_m, moved = xmem.MatrixTracker.find(g, start, forward, tol=tol, lo=0x4000_0000, hi=0xFFFF_0000)
                    if found is not None:
                        break
                    say(f"trackfoot: none moving among {n_m} within {tol:.0f} m, again")
                tracker = found
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
                clicked, walk_t0 = 0.0, time.time()
                import xmem

                camera = xmem.Camera(xmem.Guest(p.pid))
                last_say = 0.0
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
                    # The camera's own yaw, read from the 360 tables (xmem.Camera);
                    # stick angle a to the right of it moves her on bearing yaw - a
                    # (right turns lower the bearing in this world).
                    yaw = camera.yaw() if camera else None
                    travel = 1.0
                    target = 0.0 if yaw is None else (yaw - want + 180) % 360 - 180
                    # At most 70 deg off forward: pulled back she backpedals
                    # slowly; running at an angle she turns and the camera
                    # swings round behind her.
                    target = max(-70.0, min(70.0, target))
                    step_a = (target - a + 180) % 360 - 180
                    a = (a + max(-20.0, min(20.0, step_a)) + 180) % 360 - 180
                    # ls: L3 (the game's sprint, tControlMap) latches sprint while
                    # the stick is pushed, so it is clicked once, moving, not held.
                    bb = b & ~BTN["ls"]
                    if b & BTN["ls"] and not clicked and time.time() - walk_t0 > 1.0:
                        bb |= BTN["ls"]
                        clicked = time.time()
                    elif clicked and time.time() - clicked < 0.2:
                        bb |= BTN["ls"]
                    apply(bb, lt, rt, math.sin(math.radians(a)), math.cos(math.radians(a)), 0.0, 0.0)
                    if every and time.time() >= next_shot:
                        next_shot = time.time() + every
                        shot(f"{name}-{k:03d}")
                        k += 1
                    if time.time() - last_say > 1.0:
                        last_say = time.time()
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

                vmax = next((float(t[4:]) for t in w if t.startswith("max=")), None)
                w = [t for t in w if not t.startswith("max=")]
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
                    # max=<m/s>: no throttle over that speed (from the tracked trail).
                    if vmax is not None and len(getattr(tracker, "trail", [])) >= 2:
                        (t1, p1), (t2, p2) = tracker.trail[0], tracker.trail[-1]
                        if t2 > t1 and math.hypot(p2[0] - p1[0], p2[2] - p1[2]) / (t2 - t1) > vmax:
                            gas = False
                    apply(b if gas else 0, lt if gas else 0, rt if gas else 0, lx, 0.0, 0.0, 0.0)
                    if time.time() - last_log > 1.0:
                        last_log = time.time()
                        say(f"goto: ({pos[0]:.0f}, {pos[2]:.0f}) heading {hdg if hdg is None else round(hdg)} want {want:.0f} dist {dist:.0f} stick {lx:+.2f}")
                    time.sleep(0.15)
                else:
                    say(f"goto ({tx:.0f}, {tz:.0f}) TIMED OUT")
                apply()
            elif op == "if_text":
                # if_text <text> <step...>: run the step only if a fresh
                # screenshot's OCR holds the text (_ for spaces).
                if text_seen(w[1].replace("_", " "), f"if-{i:03d}"):
                    pending.append(" ".join(w[2:]))
                    say(f"if_text '{w[1]}' seen: {' '.join(w[2:])}")
                else:
                    say(f"if_text '{w[1]}' not seen")
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
