# deathprobe.py <out.txt> <car x> <car z> <seconds>: the car = the 100-health record within 1 m of (x, z)
# Read-only. Around a vehicle's death: (1) the car's state record (pos, quat, counter) on EVERY counter change (tight loop);
# (2) the car-record counter at which each new area-damage entry (pool 0x8365650C) first appears, with its transform
# (pool 0x8365808C); (3) the wheel controller's wheel+0x120 bytes per counter (controller found once by a heuristic scan:
# +0x208 == wheel count 2..6, +0x54.. wheel pointers, each wheel+0x120 byte 0/1); (4) once the car's health is 0, every
# object with vtable 0x820C7A24 (explosion action), first 0x100 bytes, scanned twice (death frame and ~0.2 s later).
import sys, struct, subprocess, time
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
import xmem
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g)
import math
cx, cz, secs = float(sys.argv[2]), float(sys.argv[3]), float(sys.argv[4])
hm = xmem.map_entries(g, 0x83656558); car = None
for k, a in hm.items():
    sa = H.state_addr(k)
    if sa:
        p = struct.unpack(">3f", g.read(sa, 12))
        if math.hypot(p[0] - cx, p[2] - cz) < 1.0 and struct.unpack(">f", g.read(a, 4))[0] == 100.0:
            car = k; break
ca = hm.get(car) if car else None
def wheels_of(car):
    """Owner chain (vehicle thread, checked in disassembly): state entry (0x83658140 map) -> P = u32(entry+0x34) (car physics,
    vtable 0x820C8560) -> C = u32(P+0x158) (vehicle core; needs u32(C+0x260) != 0) -> n = u32(C+0x208) wheels at u32(C+0x54+4i).
    Per wheel: float +0x0 = contact timer (exactly 0.4 when it has contacts this step), u32 +0x34 = raw contact count."""
    e = H.state_addr(car)
    if not e: return None, []
    P = H._u32(e + 0x34); C = H._u32(P + 0x158) if P else 0
    if not C or not H._u32(C + 0x260): return (P, C), []
    n = H._u32(C + 0x208)
    return (P, C), [w for w in (H._u32(C + 0x54 + 4 * i) for i in range(min(n, 8))) if w]
def wheel_state(ws):
    out = []
    for w in ws:
        b = g.read(w, 0x38)
        if b and len(b) == 0x38:
            out.append(f"{struct.unpack_from('>f', b, 0)[0]:.3f}/{struct.unpack_from('>I', b, 0x34)[0]}")
    return " ".join(out)
t0 = time.time()
with open(sys.argv[1], "w") as fh:
    w = lambda s: (fh.write(s + "\n"), fh.flush())
    pc, wls = wheels_of(car)
    w(f"start: car {car and hex(car)} health entry {ca and hex(ca)}; physics/core {pc and [hex(x) for x in pc if x is not None]}; wheels {[hex(x) for x in wls]}")
    seen = set(xmem.map_entries(g, 0x8365650C)); ad_keys = set(); last_c = None; dead_at = None; scans = 0
    while time.time() - t0 < secs:
        sa = H.state_addr(car); b = g.read(sa, 32) if sa else None
        if b and len(b) == 32:
            f = struct.unpack(">3fI4f", b)
            if f[3] != last_c:
                last_c = f[3]
                hv = struct.unpack(">f", g.read(ca + 4, 4))[0] if ca else float("nan")
                wb = wheel_state(wls)
                w(f"t {time.time()-t0:.4f} car counter {f[3]} pos ({f[0]:.4f},{f[1]:.4f},{f[2]:.4f}) quat ({f[4]:.5f},{f[5]:.5f},{f[6]:.5f},{f[7]:.5f}) health {hv:.2f} wheels [{wb}]")
                for k, a in xmem.map_entries(g, 0x8365650C).items():
                    if k in seen: continue
                    seen.add(k); ad_keys.add(k)
                    e = g.read(a, 0x40) or b""
                    tr = xmem.map_entries(g, 0x8365808C).get(k)
                    tp = struct.unpack(">3f", g.read(tr, 12)) if tr else None
                    w(f"  NEW area-damage {k:#x} at car counter {f[3]}: template {struct.unpack_from('>I', e, 0x30)[0]:#x} key {struct.unpack_from('>I', e, 0x2C)[0]:#x} pos {tp}")
                # every area-damage entry seen so far: its transform (pool 0x8365808C) and its attach record (table 0x83656008,
                # parent guid + offset, raw 0x30 bytes), each counter, to see whether the frag rides the car and the fx stays put
                att = xmem.map_entries(g, 0x83656008)
                for k in [kk for kk in seen if kk >> 28 == 4][-6:]:
                    if k not in ad_keys: continue
                    tr = xmem.map_entries(g, 0x8365808C).get(k)
                    tp = struct.unpack(">3f", g.read(tr, 12)) if tr else None
                    ar = att.get(k); ab = g.read(ar, 0x30) if ar else None
                    w(f"    {k:#x} pos {tp and tuple(round(c, 4) for c in tp)} attach {ab.hex() if ab else None}")
                if hv <= 0 and dead_at is None: dead_at = time.time()
        if dead_at is not None and scans < 2 and time.time() - dead_at > scans * 0.2:
            scans += 1
            for a in xmem.scan_word(g, 0x820C7A24, lo=0x40000000, hi=0x1_0000_0000, limit=16):
                d = g.read(a, 0x100) or b""
                w(f"  explosion action @ {a:#x} (scan {scans}): " + " ".join(f"{x:08x}" for x in struct.unpack(f">{len(d)//4}I", d)))
        time.sleep(0.0003)
