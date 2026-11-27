# tpoke3.py <out.txt> <x> <y> <z>: long hop with the streaming-focus candidate (her position only; Brendan 2026-11-25 15:01Z).
# Before anything: scan every copy of her position (as copylog). Stage 1A: hop 150 m short of (x, z) at y + 3 writing phantom+0xD0 AND
# 0x82BE7044 (static copy that followed with a ~4.4 s lag in copylog; 0x92BE7044 is its mirror, not written separately), re-written every
# 0.1 s for 15 s; release; settle test (within 6 m below, |dy| < 0.05 over 2 s). If 1A fails: wait 20 s for the game's recovery, then
# variant 1B = the same plus every scanned copy that still holds her PRE-hop position (the "stayed" group, found live). Stage 2 to
# (x, y + 2, z) the same way. Dry run (phantom == state record within 0.05 m, and 0x82BE7044 within 1 m of it) before every write.
import sys, struct, subprocess, time, math
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
sys.path.insert(0, "C:/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad")
import xmem, physchain as pc
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g); u32 = lambda a: pc.u32(g, a)
tx, ty, tz = map(float, sys.argv[2:5]); FOCUS = 0x82BE7044
fh = open(sys.argv[1], "w"); w = lambda s: (fh.write(s + "\n"), fh.flush())
ph = 0; tw = time.time()
while time.time() - tw < 60:
    hero = H.character(); rec, P, how = pc.lookup(g, H, hero) if hero else (None, None, None)
    ph = u32(u32(P + 0x4C) + 0x34) if P else 0
    if pc.ok_ptr(ph) and u32(ph) == 0x82051FF4 and H.update(): break
    time.sleep(0.5)
if not (pc.ok_ptr(ph) and u32(ph) == 0x82051FF4): w("ABORT: no phantom"); sys.exit(1)
time.sleep(3); hp0 = H.update()
xw = struct.unpack(">I", struct.pack(">f", hp0[0]))[0]
copies = [a for a in xmem.scan_word(g, xw, 0x40000000, 0xC0000000, 160) + xmem.scan_word(g, xw, 0x82000000, 0x90000000, 64)
          if pc.f3(g, a) and abs(pc.f3(g, a)[2] - hp0[2]) < 0.01 and abs(pc.f3(g, a)[1] - hp0[1]) < 0.01]
w(f"pre-hop position {hp0}; exact copies {len(copies)}: {[hex(a) for a in copies]}")
def dry(tag):
    cur, recp, foc = pc.f3(g, ph + 0xD0), H.update(), pc.f3(g, FOCUS)
    d = math.dist(cur, recp) if cur and recp else 1e9; df = math.dist(foc, recp) if foc and recp else 1e9
    ok = d < 0.05 and df < 1.0
    w(f"DRY RUN {tag}: phantom {cur} record {recp} diff {d:.4f}; 0x82BE7044 {foc} diff {df:.3f} -> {'OK' if ok else 'MISMATCH'}"); return ok
def hop(tag, p, extra, secs):
    if not dry(tag): w("ABORT: mismatch, nothing written"); sys.exit(1)
    targets = [ph + 0xD0, FOCUS] + extra
    w(f"WRITE {tag} {p} to {[hex(a) for a in targets]}"); t0 = time.time()
    while time.time() - t0 < secs:
        for a in targets: g.write(a, struct.pack(">3f", *p))
        time.sleep(0.1)
    w(f"released after {secs} s"); ys = []; t0 = time.time()
    while time.time() - t0 < 6:
        h = H.update(); ys.append((round(time.time() - t0, 1), round(h[1], 2) if h else None)); time.sleep(0.25)
    w(f"  settle y {ys}")
    last = [y for t, y in ys if t > 4 and y is not None]
    ok = bool(last) and p[1] - min(last) < 6 and max(last) - min(last) < 0.05
    w(f"  {tag}: {'SETTLED' if ok else 'NOT SETTLED'}"); return ok
dx, dz = hp0[0] - tx, hp0[2] - tz; L = math.hypot(dx, dz) or 1
s1 = (tx + 150 * dx / L, ty + 3, tz + 150 * dz / L)
if not hop("stage1A", s1, [], 15):
    stayed = [a for a in copies if pc.f3(g, a) and math.dist(pc.f3(g, a), hp0) < 0.05 and a not in (ph + 0xD0, FOCUS)]
    w(f"stage 1A failed; copies still at the pre-hop position (stayed group): {[hex(a) for a in stayed]}; waiting 20 s for recovery")
    time.sleep(20)
    if not hop("stage1B", s1, stayed, 15): w("ABORT after stage 1B"); sys.exit(1)
    extra = stayed
else:
    extra = []
ok2 = hop("stage2", (tx, ty + 2, tz), extra, 5)
w("DONE " + ("OK" if ok2 else "stage 2 not settled"))
