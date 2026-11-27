# hopchain3.py (rpg8): water band settle (-37 <= y <= -33 and not trending down) or stable within +-1.5 m; hopchain2: settle = stable within +-0.5 m over 2 s and above -40 (she swims in the sea near y -34.6); abort on a real fall (below -40 or falling > 1 m/s). Based on hopchain: <out.txt> <x> <z> <step m> <wait s>: chained short hops toward (x, z), her position only (phantom+0xD0; Brendan 2026-11-25
# 15:01Z). Each hop: dry run (phantom == state record within 0.05 m) -> one write of (next x, last settled y + 20, next z) -> wait <wait> s
# -> settle test (|dy| < 0.05 over 2 s, and no more than 40 m below the hop height, i.e. landed, not falling through). Abort the chain on
# a failed settle. Logs per hop: target, settled position, number of loaded state records (streaming) and the recovery anchor copies.
import sys, struct, subprocess, time, math
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
sys.path.insert(0, "C:/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad")
import xmem, physchain as pc
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g); u32 = lambda a: pc.u32(g, a)
tx, tz, step, wt = map(float, sys.argv[2:6])
fh = open(sys.argv[1], "w"); w = lambda s: (fh.write(s + "\n"), fh.flush())
ph = 0; tw = time.time()
while time.time() - tw < 60:
    hero = H.character(); rec, P, how = pc.lookup(g, H, hero) if hero else (None, None, None)
    ph = u32(u32(P + 0x4C) + 0x34) if P else 0
    if pc.ok_ptr(ph) and u32(ph) == 0x82051FF4 and H.update(): break
    time.sleep(0.5)
if not (pc.ok_ptr(ph) and u32(ph) == 0x82051FF4): w("ABORT: no phantom"); sys.exit(1)
time.sleep(3)
hop = 0; t_all = time.time()
while True:
    cur, recp = pc.f3(g, ph + 0xD0), H.update()
    d = math.dist(cur, recp) if cur and recp else 1e9
    rem = math.hypot(tx - recp[0], tz - recp[2])
    if rem < 20: w(f"ARRIVED at {tuple(round(v, 2) for v in recp)} ({rem:.1f} m from target) after {hop} hops, {time.time()-t_all:.0f} s"); break
    if d >= 0.05: w(f"ABORT hop {hop + 1}: dry run mismatch phantom {cur} record {recp} diff {d:.4f}"); sys.exit(1)
    k = min(step, rem) / rem
    p = (recp[0] + (tx - recp[0]) * k, recp[1] + 20, recp[2] + (tz - recp[2]) * k)
    hop += 1; ok = g.write(ph + 0xD0, struct.pack(">3f", *p))
    w(f"hop {hop}: from {tuple(round(v, 2) for v in recp)} write {tuple(round(v, 2) for v in p)} ok {ok} (remaining {rem:.0f} m)")
    time.sleep(wt)
    ys = []
    for _ in range(9): h = H.update(); ys.append(h[1] if h else None); time.sleep(0.25)
    last = [y for y in ys[-8:] if y is not None]
    fall_rate = (last[0] - last[-1]) / 2.0 if len(last) > 1 else 0
    in_water = bool(last) and all(-37.0 <= y <= -33.0 for y in last) and (last[-1] - last[0]) > -0.3   # swimming: in the band, not sinking
    settled = bool(last) and last[-1] > -40 and fall_rate < 1.0 and (in_water or max(last) - min(last) < 1.5)
    n = len(H.table_index(H.STATES))
    w(f"  hop {hop}: settled {settled} at {tuple(round(v, 2) for v in H.update())} (drop {p[1] - (last[-1] if last else 0):.1f} m), loaded state records {n}")
    if not settled: w(f"ABORT: hop {hop} did not settle (y samples {[round(y, 2) for y in ys if y is not None]})"); sys.exit(1)
