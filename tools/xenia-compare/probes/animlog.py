# animlog.py <out.csv> <seconds> <radius>: her + people within radius: position, health, stance/action/mode at ~20 Hz
import sys, struct, subprocess, math, time, csv
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
import xmem
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g)
here = H.update(); hero = H.character()
hm = xmem.map_entries(g, 0x83656558)
picks = [hero] + [k for k in hm if k >> 28 == 4 and not hero - 1 <= k <= hero + 8 and H.state_addr(k)
                  and math.dist(struct.unpack(">3f", g.read(H.state_addr(k), 12)), here) <= float(sys.argv[3])]
print("picks", len(picks), flush=True)
end = time.time() + float(sys.argv[2]); t0 = time.time()
with open(sys.argv[1], "w", newline="") as fh:
    wr = csv.writer(fh); wr.writerow(["t", "handle", "x", "y", "z", "health", "stance", "action", "mode"])
    while time.time() < end:
        for k in picks:
            sa = H.state_addr(k)
            if not sa: continue
            p = struct.unpack(">3f", g.read(sa, 12)); a = hm.get(k)
            hv = struct.unpack(">f", g.read(a + 4, 4))[0] if a else float("nan")
            st = H.anim_state(k) or (0, 0, 0)
            wr.writerow([f"{time.time()-t0:.3f}", f"{k:#x}", *(f"{c:.3f}" for c in p), f"{hv:.1f}", *(f"{x:#010x}" for x in st)])
        fh.flush(); time.sleep(0.04)
