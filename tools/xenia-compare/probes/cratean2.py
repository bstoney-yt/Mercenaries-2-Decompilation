import csv, math, collections
exec(open("cratean.py").read().split("F = collections")[0])
F = collections.OrderedDict()
for r in R: F.setdefault(int(r["truck_c"]), {"t": float(r["t"]), "r": r, "c": {}})["c"][r["crate"]] = r
tq = lambda r: tuple(float(r[k]) for k in ("tqx","tqy","tqz","tqw"))
def loc_of(c, cr):
    f = F[c]; r = f["r"]; T = (float(r["tx"]), float(r["ty"]), float(r["tz"])); rr = f["c"][cr]
    return inv_rot(tq(r), (float(rr["x"])-T[0], float(rr["y"])-T[1], float(rr["z"])-T[2])), float(rr["y"])
ks = list(F); near = lambda target: min(ks, key=lambda k: abs(k - target))
c0, c1, c2 = ks[0], near(400), near(1060)
r0, r1 = F[c0]["r"], F[c1]["r"]
print(f"truck c{c0} y {r0['ty']} -> c{c1} y {r1['ty']} -> c{c2} y {F[c2]['r']['ty']}")
print("settle (first logged c176 -> c400, before she drives): crate world dy, truck-frame displacement")
for cr in sorted(F[c0]["c"]):
    (a, ya), (b, yb) = loc_of(c0, cr), loc_of(c1, cr)
    print(f"  {cr} dy {yb-ya:+.3f} m, local d ({b[0]-a[0]:+.3f},{b[1]-a[1]:+.3f},{b[2]-a[2]:+.3f}) |{math.dist(a,b):.3f}|")
# when does the settle end: max crate speed per counter
for c in ks[:60:3]:
    vs = [math.sqrt(sum(float(rr[k])**2 for k in ("vx","vy","vz"))) for rr in F[c]["c"].values()]
    print("  c", c, "max crate |v|", round(max(vs), 3))
# driving: base at c2, first deviation > 5 cm
base = {cr: loc_of(c2, cr)[0] for cr in F[c2]["c"]}; first = {}; prev = None
yaw = lambda q: math.degrees(math.atan2(2*(q[3]*q[1]+q[0]*q[2]), 1-2*(q[1]**2+q[0]**2)))
for c in ks[ks.index(c2):]:
    r = F[c]["r"]; T = (float(r["tx"]), float(r["tz"])); q = tq(r)
    if prev:
        dt = (c - prev[0]) / 30; spd = math.hypot(T[0]-prev[1][0], T[1]-prev[1][1]) / dt
        yr = ((yaw(q) - yaw(prev[2]) + 180) % 360 - 180) / dt
        for cr in F[c]["c"]:
            if cr in first or cr not in base: continue
            d = math.dist(loc_of(c, cr)[0], base[cr])
            if d > 0.05: first[cr] = (c, F[c]["t"], round(d,3), round(spd,2), round(yr,1), (r["tvx"], r["tvz"]), (r["twy"]))
    prev = (c, T, q)
print("driving, first slide > 5 cm from the c%d rest pose:" % c2)
for cr, v in sorted(first.items(), key=lambda kv: kv[1][0]): print("  ", cr, *v)
print("no slide:", [c for c in base if c not in first])
