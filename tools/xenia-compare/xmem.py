"""Read Xenia's guest memory from outside (read-only, ReadProcessMemory).

Xenia Canary reserves the 360's 4 GB virtual address space at a fixed host base
(0x100000000), so guest address A is host 0x100000000 + A; values are big
endian. This finds things like the hero's position by scanning for float
triplets near a known point, then reads them back while a route plays.

    python -I xmem.py scan <x> <y> <z> [--tol 3] [--pid N]   # candidate guest addresses
    python -I xmem.py watch <guest_addr> [<guest_addr>...]   # print vec3s every 0.5 s
"""

from __future__ import annotations

import argparse
import ctypes
import ctypes.wintypes as wt
import struct
import subprocess
import sys
import time

import numpy as np

HOST_BASE = 0x1_0000_0000
GUEST_SIZE = 0x1_0000_0000
k32 = ctypes.WinDLL("kernel32", use_last_error=True)
k32.OpenProcess.restype = wt.HANDLE
k32.ReadProcessMemory.argtypes = [wt.HANDLE, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_size_t, ctypes.POINTER(ctypes.c_size_t)]
k32.VirtualQueryEx.argtypes = [wt.HANDLE, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_size_t]
k32.VirtualQueryEx.restype = ctypes.c_size_t


class MBI(ctypes.Structure):
    _fields_ = [("BaseAddress", ctypes.c_uint64), ("AllocationBase", ctypes.c_uint64),
                ("AllocationProtect", wt.DWORD), ("PartitionId", wt.WORD), ("RegionSize", ctypes.c_uint64),
                ("State", wt.DWORD), ("Protect", wt.DWORD), ("Type", wt.DWORD)]


MEM_COMMIT, PAGE_NOACCESS, PAGE_GUARD = 0x1000, 0x01, 0x100


def xenia_pid(name: str = "m2-alpha-xenia") -> int:
    out = subprocess.run(["powershell", "-NoProfile", "-c", f"(Get-Process {name}).Id"],
                         capture_output=True, text=True).stdout.split()
    if not out:
        raise SystemExit(f"no {name} process")
    return int(out[0])


class Guest:
    def __init__(self, pid: int) -> None:
        self.h = k32.OpenProcess(0x0010 | 0x0400, False, pid)  # VM_READ | QUERY_INFORMATION
        if not self.h:
            raise OSError(ctypes.get_last_error(), "OpenProcess")

    def read(self, guest: int, n: int) -> bytes | None:
        buf = ctypes.create_string_buffer(n)
        got = ctypes.c_size_t()
        ok = k32.ReadProcessMemory(self.h, HOST_BASE + guest, buf, n, ctypes.byref(got))
        return buf.raw[: got.value] if ok else None

    def vec3(self, guest: int) -> tuple[float, float, float] | None:
        b = self.read(guest, 12)
        return struct.unpack(">3f", b) if b and len(b) == 12 else None

    def regions(self, lo: int = 0, hi: int = 0xA000_0000):
        """Committed, readable guest regions in [lo, hi) as (guest, size)."""
        a = HOST_BASE + lo
        m = MBI()
        while a < HOST_BASE + hi:
            if not k32.VirtualQueryEx(self.h, a, ctypes.byref(m), ctypes.sizeof(m)):
                break
            size = m.RegionSize
            if m.State == MEM_COMMIT and not m.Protect & (PAGE_NOACCESS | PAGE_GUARD):
                yield m.BaseAddress - HOST_BASE, size
            a = m.BaseAddress + size

    def scan_vec3(self, target, tol: float, lo: int = 0, hi: int = 0xA000_0000, limit: int = 64):
        """Guest addresses (4-aligned) of 3 big-endian floats within tol of target."""
        tx, ty, tz = target
        hits = []
        for g, size in self.regions(lo, hi):
            for off in range(0, size, 1 << 24):
                n = min(1 << 24, size - off)
                b = self.read(g + off, n)
                if not b or len(b) < 12:
                    continue
                f = np.frombuffer(b[: len(b) // 4 * 4], dtype=">f4")
                with np.errstate(invalid="ignore"):
                    mx = np.abs(f[:-2] - tx) <= tol
                    idx = np.nonzero(mx)[0]
                    for i in idx:
                        if abs(f[i + 1] - ty) <= tol and abs(f[i + 2] - tz) <= tol:
                            hits.append(g + off + 4 * int(i))
                            if len(hits) >= limit:
                                return hits
        return hits


class Tracker:
    """Follows one object's position (say the hero's boat) through buffers
    that many objects share: each update scans a few small guest areas for
    vec3s near the last fix and keeps the one nearest where it should be."""

    AREAS = ((0x7000_0000, 0x0100_0000), (0x8300_0000, 0x0100_0000))

    def __init__(self, g: Guest, start, reach: float = 30.0) -> None:
        self.g, self.pos, self.vel, self.t = g, tuple(start), (0.0, 0.0, 0.0), time.time()
        self.reach = reach
        self.prev: dict[int, tuple] = {}

    def candidates(self, around, tol):
        """(guest address, vec3) of the vec3s within tol of `around`."""
        out = []
        for lo, n in self.AREAS:
            for off in range(0, n, 1 << 22):
                b = self.g.read(lo + off, 1 << 22)
                if not b:
                    continue
                f = np.frombuffer(b[: len(b) // 4 * 4], dtype=">f4")
                with np.errstate(invalid="ignore"):
                    ix = np.nonzero(np.abs(f[:-2] - around[0]) <= tol)[0]
                    for i in ix:
                        if abs(f[i + 2] - around[2]) <= tol and abs(f[i + 1] - around[1]) <= 12:
                            out.append((lo + off + 4 * int(i), (float(f[i]), float(f[i + 1]), float(f[i + 2]))))
        return out

    def update(self):
        """The new fix: the candidate nearest the predicted place, where copies
        that have not changed for a few updates (stale buffers) count as 25 m
        further away, so a moving object is not lost to an old copy."""
        now = time.time()
        dt = now - self.t
        pred = tuple(p + v * dt for p, v in zip(self.pos, self.vel))
        cands = self.candidates(pred, self.reach)
        if not cands:
            return None
        seen = {}
        for addr, v in cands:
            old, still = self.prev.get(addr, (None, 0))
            seen[addr] = (v, still + 1 if old == v else 0)
        self.prev = seen

        def score(item):
            addr, c = item
            d = ((c[0] - pred[0]) ** 2 + (c[2] - pred[2]) ** 2) ** 0.5
            return d + (25.0 if seen[addr][1] >= 3 else 0.0)

        _, best = min(cands, key=score)
        if dt > 0.05:
            nv = tuple((b - p) / dt for b, p in zip(best, self.pos))
            self.vel = tuple(0.75 * a + 0.25 * b for a, b in zip(self.vel, nv))
        self.pos, self.t = best, now
        return best

    def heading(self) -> float | None:
        """Degrees from +z towards +x of the motion, or None when still."""
        import math

        if self.vel[0] ** 2 + self.vel[2] ** 2 < 1.0:
            return None
        return math.degrees(math.atan2(self.vel[0], self.vel[2]))


def world_matrix(g: Guest, t_addr: int):
    """The row-major 4x4 world matrix whose translation row starts at t_addr:
    (rows 0..2, translation), or None if it does not look like one (unit
    rows with 0 in their 4th column, w = 1)."""
    b = g.read(t_addr - 0x30, 0x40)
    if not b or len(b) < 0x40:
        return None
    f = struct.unpack(">16f", b)
    rows = (f[0:3], f[4:7], f[8:11])
    ok = all(abs(sum(x * x for x in r) - 1) < 0.03 for r in rows)
    ok = ok and abs(f[3]) + abs(f[7]) + abs(f[11]) < 1e-3 and abs(f[15] - 1) < 1e-3
    return (rows, f[12:15]) if ok else None


class MatrixTracker:
    """Follows a vehicle by its world matrices: position from the translation
    row, heading from row 2 (its forward axis, measured on the speedboat).
    The game keeps several copies (render, physics...) and some are only
    refreshed now and then, so it reads all the copies that moved when it
    was found and uses the one that changed most recently."""

    def __init__(self, g: Guest, addrs) -> None:
        self.g, self.addrs = g, list(addrs)
        self.addr = self.addrs[0]
        self.last = {a: None for a in self.addrs}
        self.changed = {a: 0.0 for a in self.addrs}
        self.rows = None
        self.pos = None
        self.trail: list[tuple[float, tuple]] = []

    @classmethod
    def find(cls, g: Guest, start, press, tol: float = 15.0):
        """The world matrices within tol of start that move forward along
        their own row 2 while `press()` runs (e.g. the throttle)."""
        cands = []
        for h in g.scan_vec3(start, tol, lo=0x4000_0000, hi=0xFFFF_0000, limit=100000):
            m = world_matrix(g, h)
            if m:
                cands.append((h, m))
        press()
        movers = []
        for h, (rows, t0) in cands:
            m = world_matrix(g, h)
            if not m:
                continue
            d = [a - b for a, b in zip(m[1], t0)]
            fwd = d[0] * rows[2][0] + d[2] * rows[2][2]
            side = abs(d[0] * rows[2][2] - d[2] * rows[2][0])
            # 1.2 s of throttle is well under 30 m; buffers reused for other
            # things jump much further, or sideways.
            if 0.5 < fwd < 30 and side < 0.5 * fwd + 1:
                movers.append((fwd, h))
        movers.sort(reverse=True)
        if not movers:
            return None, len(cands), 0.0
        return cls(g, [h for _, h in movers[:16]]), len(cands), movers[0][0]

    def update(self):
        now = time.time()
        fixes = {}
        for a in self.addrs:
            m = world_matrix(self.g, a)
            if not m:
                continue
            fixes[a] = m
            if m[1] != self.last[a]:
                self.last[a], self.changed[a] = m[1], now
        if self.rows is not None and self.pos is not None:
            fixes = {a: m for a, m in fixes.items()
                     if (m[1][0] - self.pos[0]) ** 2 + (m[1][2] - self.pos[2]) ** 2 < 900}
        if not fixes:
            return None
        self.addr = max(fixes, key=lambda a: self.changed[a])
        rows, t = fixes[self.addr]
        self.rows, self.pos = rows, t
        self.trail = [(ts, p) for ts, p in self.trail if now - ts < 1.0] + [(now, t)]
        return t

    def heading(self) -> float | None:
        """From the last second's travel when that is over 2 m (some copies'
        rotation goes stale while their position still updates), else from
        the matrix's row 2."""
        import math

        if len(self.trail) >= 2:
            (_, a), (_, b) = self.trail[0], self.trail[-1]
            if math.hypot(b[0] - a[0], b[2] - a[2]) > 2.0:
                return math.degrees(math.atan2(b[0] - a[0], b[2] - a[2]))
        if not self.rows:
            return None
        f = self.rows[2]
        return math.degrees(math.atan2(f[0], f[2]))


def main() -> int:
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("scan")
    s.add_argument("xyz", nargs=3, type=float)
    s.add_argument("--tol", type=float, default=3.0)
    s.add_argument("--pid", type=int)
    w = sub.add_parser("watch")
    w.add_argument("addr", nargs="+")
    w.add_argument("--pid", type=int)
    w.add_argument("--n", type=int, default=20)
    a = ap.parse_args()
    g = Guest(a.pid or xenia_pid())
    if a.cmd == "scan":
        t = time.time()
        hits = g.scan_vec3(a.xyz, a.tol)
        for h in hits:
            print(f"{h:#010x} {g.vec3(h)}")
        print(f"{len(hits)} hits in {time.time() - t:.1f} s", file=sys.stderr)
    else:
        addrs = [int(x, 16) for x in a.addr]
        for _ in range(a.n):
            print(" | ".join(f"{x:#x} {tuple(round(v, 2) for v in g.vec3(x) or ())}" for x in addrs), flush=True)
            time.sleep(0.5)
    return 0


if __name__ == "__main__":
    sys.exit(main())
