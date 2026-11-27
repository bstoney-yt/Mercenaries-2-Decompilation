# tpoke.py <out.txt> <x> <y> <z> <wait s> [--dry]: RUNTIME DATA ONLY, her position only (Brendan's approval 2026-11-25).
# Her character proxy = store 0x836571D4 record P (vtable 0x820C7E48) -> inline wrapper P+0x4C -> +0x34 = Havok caching shape phantom
# (vtable 0x82051FF4; rotation rows +0xA0/+0xB0/+0xC0, translation +0xD0). In Havok's character proxy the phantom translation is the
# position the proxy integrates from, so the ONE write is the 3 floats at phantom+0xD0. Dry run first (always): read +0xD0 and her
# state-record position; abort unless they agree within 0.05 m. With --dry nothing is written. Then logs her record and +0xD0 for <wait> s.
import sys, struct, subprocess, time, math
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
sys.path.insert(0, "C:/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad")
import xmem, physchain as pc
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g); u32 = lambda a: pc.u32(g, a)
tx, ty, tz, wt = map(float, sys.argv[2:6]); dry = "--dry" in sys.argv
with open(sys.argv[1], "w") as fh:
    w = lambda s: (fh.write(s + "\n"), fh.flush())
    hero = H.character(); rec, P, how = pc.lookup(g, H, hero)
    ph = u32(u32(P + 0x4C) + 0x34) if P else 0
    w(f"hero {hero and hex(hero)} P {P and hex(P)} vt {u32(P) if P else 0:#x} phantom {ph:#x} vt {u32(ph) if pc.ok_ptr(ph) else 0:#x} (want 0x820c7e48 / 0x82051ff4)")
    if not pc.ok_ptr(ph) or u32(P) != 0x820C7E48 or u32(ph) != 0x82051FF4: w("ABORT: proxy phantom not found"); sys.exit()
    cur = pc.f3(g, ph + 0xD0); recp = H.update()
    d = math.dist(cur, recp) if cur and recp else 1e9
    w(f"DRY RUN: phantom+0xD0 {cur} state record {recp} diff {d:.4f} m -> {'OK' if d < 0.05 else 'MISMATCH'}")
    if d >= 0.05: w("ABORT: dry run mismatch, nothing written"); sys.exit()
    if dry: w(f"DRY ONLY: would write phantom {ph:#x}+0xD0 = ({tx}, {ty}, {tz})"); sys.exit()
    ok = g.write(ph + 0xD0, struct.pack(">3f", tx, ty, tz)); w(f"WRITE phantom+0xD0 = ({tx}, {ty}, {tz}) ok {ok}")
    t0 = time.time()
    while time.time() - t0 < wt:
        h = H.update(); w(f"t {time.time()-t0:.1f} record {h and tuple(round(v, 2) for v in h)} phantom {tuple(round(v, 2) for v in (pc.f3(g, ph + 0xD0) or ()))}"); time.sleep(0.5)
