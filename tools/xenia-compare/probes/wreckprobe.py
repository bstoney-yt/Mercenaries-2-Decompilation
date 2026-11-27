# wreckprobe.py <out.txt> <car x> <car z> <seconds>: read-only (Jennifer, death2 shot kill). The Phoenix wreck's bodies around the death
# (Z:\mercs2\scratch\vehicles\wreck-bodies-xenia-ask.md). Car found by its parked spot. Tight polling from the death counter - 1 to
# + 4 (buffered, minimal reads), then one row per counter to + 11. Per sample: car counter, steps mgr+0x23D4, n 0x831334AC, subdt
# 0x831334A8, world+0x80/+0x84, P+0x48, P+0x50 (count), P+0x54 (array flag), and for each body i < count: wrapper w
# (P+0x54 ? u32(u32(P+0x4C)+4i) : u32(P+0x4C)), B = u32(w+0x38), B+0x1A0 v, inverse mass w-component B+0x19C, position B+0x110,
# w+0x24 (part index), w+0x28 (part record pointer) and that record's first 0x20 bytes.
import sys, struct, subprocess, time, math
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
sys.path.insert(0, "C:/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad")
import xmem, physchain as pc
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g)
cx, cz, secs = float(sys.argv[2]), float(sys.argv[3]), float(sys.argv[4])
u32 = lambda a: pc.u32(g, a); f32 = lambda a: struct.unpack(">f", g.read(a, 4) or bytes(4))[0]
car = ca = None; t0 = time.time()
while car is None and time.time() - t0 < 60:
    for k, a in xmem.map_entries(g, 0x83656558).items():
        sa = H.state_addr(k)
        if sa:
            p = struct.unpack(">3f", g.read(sa, 12))
            if math.hypot(p[0] - cx, p[2] - cz) < 1.5 and struct.unpack(">f", g.read(a, 4))[0] == 100.0: car, ca = k, a; break
    if car is None: time.sleep(0.3)
t0 = time.time(); mgr = u32(0x831334C8)
def snap(P):
    n, arr, lst = u32(P + 0x50), u32(P + 0x54), u32(P + 0x4C)
    bodies = []
    for i in range(min(n, 8)):
        w = u32(u32(lst + 4 * i)) if False else (u32(lst + 4 * i) if arr else (lst if i == 0 else 0))
        if not pc.ok_ptr(w): bodies.append((i, w, 0, None, None, None, 0, 0, b"")); continue
        B = u32(w + 0x38)
        v = g.read(B + 0x1A0, 12) if pc.ok_ptr(B) else None; pos = g.read(B + 0x110, 12) if pc.ok_ptr(B) else None
        im = g.read(B + 0x19C, 4) if pc.ok_ptr(B) else None; rec = u32(w + 0x28)
        bodies.append((i, w, B, v, pos, im, u32(w + 0x24), rec, (g.read(rec, 0x20) or b"") if pc.ok_ptr(rec) else b""))
    return n, arr, lst, bodies
def fmt3(b): return "None" if not b or len(b) < 12 else "(" + ",".join(f"{x:.4f}" for x in struct.unpack(">3f", b)) + ")"
def line(t, c, P, n, arr, lst, bodies, wd):
    s = (f"t {t:.6f} c {c} steps {u32(mgr + 0x23D4)} n {u32(0x831334AC)} subdt {f32(0x831334A8):.5f} world+0x80 {u32(wd + 0x80) if wd else 0} "
         f"+0x84 {u32(wd + 0x84) if wd else 0} P+0x48 {u32(P + 0x48):#x} count P+0x50 {n} P+0x54 {arr:#x} list {lst:#x}")
    for i, w, B, v, pos, im, part, rec, rb in bodies:
        mass = (1 / struct.unpack(">f", im)[0]) if im and struct.unpack(">f", im)[0] else 0.0
        s += f" | body{i} w {w:#x} B {B:#x} v {fmt3(v)} pos {fmt3(pos)} mass {mass:.1f} part w+0x24 {part:#x} rec w+0x28 {rec:#x} [{rb.hex()}]"
    return s
with open(sys.argv[1], "w") as fh:
    entry = H.state_addr(car) if car else None
    rec, P, how = pc.lookup(g, H, car) if car else (None, None, None)
    fh.write(f"car {car and hex(car)} P {P and hex(P)} mgr {mgr:#x}\n"); fh.flush()
    if not P: sys.exit()
    last = None; dead = None; buf = []; ring = []
    while time.time() - t0 < secs and entry:
        c = u32(entry + 0xC); hv = f32(ca + 4)
        n, arr, lst, bodies = snap(P)
        wd = u32(bodies[0][2] + 8) if bodies and pc.ok_ptr(bodies[0][2]) else 0
        if dead is None:
            if c != last:
                last = c; ring = (ring + [line(time.time() - t0, c, P, n, arr, lst, bodies, wd)])[-2:]
            if hv <= 0: dead = c; fh.write("\n".join("pre " + r for r in ring) + "\n")
            continue
        if c <= dead + 4:
            buf.append(line(time.time() - t0, c, P, n, arr, lst, bodies, wd)); continue
        if buf: fh.write("\n".join("tight " + r for r in buf) + "\n"); buf = []
        if c != last:
            last = c; fh.write("row " + line(time.time() - t0, c, P, n, arr, lst, bodies, wd) + "\n"); fh.flush()
        if c > dead + 11: break
    if buf: fh.write("\n".join("tight " + r for r in buf) + "\n")
