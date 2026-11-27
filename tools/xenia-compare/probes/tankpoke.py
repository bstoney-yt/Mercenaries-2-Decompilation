# tankpoke.py <out.txt> <guid hex> <dy> <secs>: RUNTIME DATA ONLY (Brendan 2026-11-25 15:01Z: full approval for Xenia work; coordinator:
# tank position poke). Lifts the vehicle's rigid bodies by dy: for every body B of its store record P (inline/pointer list), shift the
# Havok motion translation B+0x110 and swept centres B+0x120 / B+0x130 by (0, dy, 0); nothing else written (velocity untouched).
# Dry run first: the vehicle's state-record position must equal body 0's B+0x110 within 0.05 m, every body a rigid body (B+0x110 finite).
# Then logs, on every physics-manager step (mgr+0x23D4), each body's B+0x110 y and B+0x1A0 for <secs> s.
import sys, struct, subprocess, time, math
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
sys.path.insert(0, "C:/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad")
import xmem, physchain as pc
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g); u32 = lambda a: pc.u32(g, a)
gd, dy, secs = int(sys.argv[2], 16), float(sys.argv[3]), float(sys.argv[4])
with open(sys.argv[1], "w") as fh:
    w = lambda s: (fh.write(s + "\n"), fh.flush())
    rec, P, how = pc.lookup(g, H, gd); bl = pc.bodies(g, P)[3] if P else []
    Bs = [B for _, B in bl if pc.ok_ptr(B)]
    w(f"vehicle {gd:#x} P {P and hex(P)} vt {u32(P) if P else 0:#x} bodies {[hex(b) for b in Bs]}")
    sa = H.state_addr(gd); rp = struct.unpack(">3f", g.read(sa, 12)) if sa else None
    b0 = pc.f3(g, Bs[0] + 0x110) if Bs else None
    ok = bool(Bs) and rp and b0 and math.dist(rp, b0) < 0.05 and all(pc.f3(g, B + 0x110) and all(math.isfinite(x) for x in pc.f3(g, B + 0x110)) for B in Bs)
    w(f"DRY RUN: state record {rp} body0 B+0x110 {b0} diff {math.dist(rp, b0) if rp and b0 else None} -> {'OK' if ok else 'MISMATCH/NOT RIGID'}")
    for B in Bs: w(f"  body {B:#x} +0x110 {pc.f3(g, B + 0x110)} +0x120 {pc.f3(g, B + 0x120)} +0x130 {pc.f3(g, B + 0x130)} v {pc.f3(g, B + 0x1A0)} invmass {pc.f3(g, B + 0x194)}")
    if not ok: w("ABORT: nothing written"); sys.exit()
    for B in Bs:
        for off in (0x110, 0x120, 0x130):
            q = pc.f3(g, B + off); w(f"  WRITE {B:#x}+{off:#x} y {q[1]:.4f} -> {q[1] + dy:.4f} ok {g.write(B + off + 4, struct.pack('>f', q[1] + dy))}")
    mgr = u32(0x831334C8); t0 = time.time(); last = None
    while time.time() - t0 < secs:
        st = u32(mgr + 0x23D4)
        if st == last: continue
        last = st
        row = f"t {time.time()-t0:.4f} step {st} n {u32(0x831334AC)} subdt {struct.unpack('>f', g.read(0x831334A8, 4))[0]:.5f} rec y {struct.unpack('>f', g.read(sa + 4, 4))[0]:.4f}"
        for i, B in enumerate(Bs):
            p = pc.f3(g, B + 0x110); v = pc.f3(g, B + 0x1A0)
            row += f" | b{i} y {p[1]:.4f} v ({v[0]:.3f},{v[1]:.4f},{v[2]:.3f})"
        w(row)
