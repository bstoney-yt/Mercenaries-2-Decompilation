# phys8probe.py <out.txt> <car x> <car z> <seconds>: read-only, #728 settling read (Jennifer, death2 shot kill). Car found by its parked
# spot. Until health 0: one row per car counter (pos, v, mass 1/w at B+0x190, world+0x80/+0x84, step count, n 0x831334AC, substep dt
# 0x831334A8, frame dt). From health 0 for 4 counters: a TIGHT loop with only these reads per sample, buffered in memory: time, car
# counter, step count, world+0x80, world+0x84, the current body (re-resolved when P's list changes) v, wheel-list length; written after.
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
with open(sys.argv[1], "w") as fh:
    w = lambda s: fh.write(s + "\n")
    entry = H.state_addr(car) if car else None
    rec, P, how = pc.lookup(g, H, car) if car else (None, None, None)
    w(f"car {car and hex(car)} P {P and hex(P)} mgr {mgr:#x}"); fh.flush()
    if not P: sys.exit()
    last = None; buf = []; dead = None
    def body():
        bl = pc.bodies(g, P)[3]; return bl[0][1] if bl else 0
    B = body()
    while time.time() - t0 < secs and entry:
        hv = f32(ca + 4)
        if dead is None:
            c = u32(entry + 0xC)
            if c != last:
                last = c; B = body(); wd = u32(B + 8)
                w(f"c {c} t {time.time()-t0:.5f} hp {hv:.2f} B {B:#x} v {pc.f3(g, B + 0x1A0)} mass {1 / ((pc.f3(g, B + 0x194) or (0, 0, 1e-9))[2] or 1e-9):.1f} "
                  f"world {wd:#x} +0x80 {u32(wd + 0x80)} +0x84 {u32(wd + 0x84)} steps {u32(mgr + 0x23D4)} n {u32(0x831334AC)} subdt {f32(0x831334A8):.5f} frame dt {f32(0x82B7C730):.5f}")
            if hv <= 0: dead = c; fh.flush()
            continue
        # tight phase: minimal reads, buffered
        c = u32(entry + 0xC)
        if c > dead + 4: break
        Bn = u32(u32(P + 0x4C) + 0x38) if not u32(P + 0x54) else u32(u32(u32(P + 0x4C)) + 0x38)
        if not pc.ok_ptr(Bn): Bn = body()
        wd = u32(Bn + 8)
        buf.append((time.time() - t0, c, u32(mgr + 0x23D4), u32(wd + 0x80), u32(wd + 0x84), Bn, g.read(Bn + 0x1A0, 12), u32(0x831334AC), f32(0x831334A8)))
    for t, c, st, q80, q84, Bn, vb, n, sdt in buf:
        v = struct.unpack(">3f", vb) if vb and len(vb) == 12 else None
        w(f"tight t {t:.6f} c {c} steps {st} n {n} subdt {sdt:.5f} world+0x80 {q80} +0x84 {q84} B {Bn:#x} v {v and tuple(round(x, 4) for x in v)}")
    B = body(); w(f"after: B {B:#x} mass 1/w(B+0x19C) {1 / (f32(B + 0x19C) or 1e-9):.1f} samples {len(buf)}")
