# tpoke.py <out.txt> <x> <y> <z> <wait s>: RUNTIME DATA ONLY (no code patching). Moves the hero's character-proxy body (store
# 0x836571D4 record -> P (vtable 0x820C7E48) -> body B): Havok motion translation B+0x110 and swept centres B+0x120 / B+0x130 shifted by
# the same delta, linear velocity B+0x1A0 zeroed; then logs her state-record position every 0.5 s for <wait> s to show whether she stays.
import sys, struct, subprocess, time
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
sys.path.insert(0, "C:/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad")
import xmem, physchain as pc
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g)
tx, ty, tz, wt = map(float, sys.argv[2:6])
with open(sys.argv[1], "w") as fh:
    w = lambda s: (fh.write(s + "\n"), fh.flush())
    hero = H.character(); rec, P, how = pc.lookup(g, H, hero)
    bl = pc.bodies(g, P)[3] if P else []; B = bl[0][1] if bl else 0
    w(f"hero {hero and hex(hero)} P {P and hex(P)} vt {pc.u32(g, P) if P else 0:#x} B {B:#x} (want P vt 0x820c7e48)")
    if not pc.ok_ptr(B) or pc.u32(g, P) != 0x820C7E48: w("ABORT: proxy body not found"); sys.exit()
    p0 = pc.f3(g, B + 0x110); d = (tx - p0[0], ty - p0[1], tz - p0[2]); w(f"before B+0x110 {p0} delta {d}")
    for off in (0x110, 0x120, 0x130):
        q = pc.f3(g, B + off)
        if q: w(f"  write B+{off:#x} {q} -> ok {g.write(B + off, struct.pack('>3f', q[0] + d[0], q[1] + d[1], q[2] + d[2]))}")
    w(f"  zero v ok {g.write(B + 0x1A0, struct.pack('>3f', 0, 0, 0))}")
    t0 = time.time()
    while time.time() - t0 < wt:
        h = H.update(); w(f"t {time.time()-t0:.1f} record {h and tuple(round(v, 2) for v in h)} body {pc.f3(g, B + 0x110)}"); time.sleep(0.5)
