# tpoke2.py <out.txt> <x> <y> <z> [hold s]: two-stage position poke (her position only; runtime data; Brendan 2026-11-25).
# Stage 1: hop to 150 m short of (x, z) on the line from her current position, at height y + 3, and HOLD that phantom position (rewrite
# +0xD0 every 0.1 s) for <hold> s while the area streams in; release; she must settle within 6 m below the hold height and be stable
# (|dy| < 0.05 m over 2 s), else abort. Stage 2: hop to (x, ground1 + (y - ground1)... ) i.e. (x, y + 2, z), hold 3 s, release, same
# settle test. A dry-run check (phantom+0xD0 == state record within 0.05 m) precedes every write; abort and report on any mismatch.
import sys, struct, subprocess, time, math
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
sys.path.insert(0, "C:/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad")
import xmem, physchain as pc
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g); u32 = lambda a: pc.u32(g, a)
tx, ty, tz = map(float, sys.argv[2:5]); hold = float(sys.argv[5]) if len(sys.argv) > 5 else 15
fh = open(sys.argv[1], "w"); w = lambda s: (fh.write(s + "\n"), fh.flush())
ph = 0; tw = time.time()
while time.time() - tw < 60:
    hero = H.character(); rec, P, how = pc.lookup(g, H, hero) if hero else (None, None, None)
    ph = u32(u32(P + 0x4C) + 0x34) if P else 0
    if pc.ok_ptr(ph) and u32(ph) == 0x82051FF4 and H.update(): break
    time.sleep(0.5)
w(f"phantom {ph:#x} (waited {time.time()-tw:.1f} s)")
if not (pc.ok_ptr(ph) and u32(ph) == 0x82051FF4): w("ABORT: no phantom"); sys.exit(1)
def dry(tag):
    cur, recp = pc.f3(g, ph + 0xD0), H.update(); d = math.dist(cur, recp) if cur and recp else 1e9
    w(f"DRY RUN {tag}: phantom {cur} record {recp} diff {d:.4f} -> {'OK' if d < 0.05 else 'MISMATCH'}"); return d < 0.05
def put(p): return g.write(ph + 0xD0, struct.pack(">3f", *p))
def hop(tag, p, secs):
    if not dry(tag): w("ABORT: mismatch, nothing written"); sys.exit(1)
    w(f"WRITE {tag} {p} ok {put(p)}"); t0 = time.time()
    while time.time() - t0 < secs: put(p); time.sleep(0.1)     # hold while collision streams in
    w(f"released after {secs} s hold")
    ys = []; t0 = time.time()
    while time.time() - t0 < 6:
        h = H.update(); ys.append((time.time() - t0, h[1] if h else None)); time.sleep(0.25)
    w(f"  settle y: {[(round(t, 1), round(y, 2)) for t, y in ys if y is not None]}")
    last = [y for t, y in ys if t > 4 and y is not None]
    ok = last and p[1] - min(last) < 6 and max(last) - min(last) < 0.05
    w(f"  {tag}: {'SETTLED' if ok else 'NOT SETTLED (fell or moving)'} final y {last[-1] if last else None}")
    return ok, (last[-1] if last else None)
cur = H.update(); dx, dz = cur[0] - tx, cur[2] - tz; L = math.hypot(dx, dz) or 1
s1 = (tx + 150 * dx / L, ty + 3, tz + 150 * dz / L)
ok1, g1 = hop("stage1", s1, hold)
if not ok1: w("ABORT after stage 1"); sys.exit(1)
ok2, g2 = hop("stage2", (tx, ty + 2, tz), 3)
w("DONE " + ("OK" if ok2 else "stage 2 not settled"))
