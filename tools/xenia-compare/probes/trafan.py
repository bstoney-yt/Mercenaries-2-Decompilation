import csv, math, collections, sys
R = list(csv.DictReader(open("Z:/mercs2/scratch/xenia-compare/traffic1/contact.txt")))
def parse(s):
    out = {}
    for e in s.split(";"):
        if not e: continue
        i, v = e.split(":"); f = list(map(float, v.split(",")))
        out[int(i)] = f
    return out
rows = [(float(r["t"]), int(r["frame"]), float(r["dt"]), float(r["hp"]), (float(r["hx"]), float(r["hy"]), float(r["hz"])), parse(r["near"])) for r in R]
# her own records: present and within 2 m in > 80% of frames
cnt = collections.Counter(); 
for r in rows:
    for i, f in r[5].items():
        if math.dist(f[:3], r[4]) < 2.0: cnt[i] += 1
own = {i for i, c in cnt.items() if c > 0.8 * len(rows)}
print("frames", len(rows), "own records", sorted(own), "hp start", rows[0][3], "end", rows[-1][3])
yaw = lambda q: math.degrees(math.atan2(2*(q[3]*q[1]+q[0]*q[2]), 1-2*(q[1]**2+q[0]**2)))
dmg = []
for a, b in zip(rows, rows[1:]):
    loss = a[3] - b[3]
    if loss <= 0.01: continue
    dfr = max(1, b[1] - a[1]); dt = dfr / 30.0
    hv = tuple((b[4][k] - a[4][k]) / dt for k in range(3))
    best = None
    for i, f in b[5].items():
        if i in own or i not in a[5]: continue
        d = math.dist(f[:3], b[4]); g0 = a[5][i]
        v = tuple((f[k] - g0[k]) / dt for k in range(3))
        yr = ((yaw(f[3:7]) - yaw(g0[3:7]) + 180) % 360 - 180) / dt
        if best is None or d < best[0]: best = (d, i, v, yr, f[:3])
    phase = "still" if b[0] < 320 else "walk"
    dmg.append((b[0], phase, loss, hv, best))
print("damaging frames", len(dmg), "by phase", collections.Counter(d[1] for d in dmg))
for t, ph, loss, hv, best in dmg[:200]:
    if best is None: print(f" t {t:7.2f} {ph} loss {loss:.2f} her v ({hv[0]:.1f},{hv[2]:.1f}) |{math.hypot(hv[0],hv[2]):.1f}| no vehicle"); continue
    d, i, v, yr, p = best
    print(f" t {t:7.2f} {ph} loss {loss:5.2f} her |v| {math.hypot(hv[0],hv[2]):4.1f} | rec {i} d {d:.2f} v ({v[0]:.1f},{v[1]:.1f},{v[2]:.1f}) |{math.hypot(v[0],v[2]):.1f}| yaw rate {yr:.0f} deg/s, offset ({p[0]-rows[0][4][0]:.1f})")
