import csv, math, collections
R = list(csv.DictReader(open("Z:/mercs2/scratch/xenia-compare/crates2/crates.csv")))
def inv_rot(q, d):
    qx, qy, qz, qw = q; ux, uy, uz = -qx, -qy, -qz
    tx = 2*(uy*d[2]-uz*d[1]); ty = 2*(uz*d[0]-ux*d[2]); tz = 2*(ux*d[1]-uy*d[0])
    return (d[0]+qw*tx+(uy*tz-uz*ty), d[1]+qw*ty+(uz*tx-ux*tz), d[2]+qw*tz+(ux*ty-uy*tx))
F = collections.OrderedDict()
for r in R: F.setdefault(int(r["truck_c"]), {"t": float(r["t"]), "r": r, "c": {}})["c"][r["crate"]] = r
fr = list(F.items()); print("frames", len(fr), "counters", fr[0][0], fr[-1][0])
# truck speed / yaw rate from positions; body v as logged
prev = None; base = {}; first_slide = {}; first_leave = {}; start_move = None
tq = lambda r: tuple(float(r[k]) for k in ("tqx","tqy","tqz","tqw"))
yaw = lambda q: math.degrees(math.atan2(2*(q[3]*q[1]+q[0]*q[2]), 1-2*(q[1]**2+q[0]**2)))
rows = []
for c, f in fr:
    r = f["r"]; T = (float(r["tx"]), float(r["ty"]), float(r["tz"])); q = tq(r)
    spd = yr = None
    if prev:
        dc = c - prev[0]; dt = dc / 30.0
        spd = math.hypot(T[0]-prev[1][0], T[2]-prev[1][2]) / dt
        dy = (yaw(q) - yaw(prev[2]) + 180) % 360 - 180; yr = dy / dt
    if spd and spd > 0.3 and start_move is None: start_move = c
    for cr, rr in f["c"].items():
        d = (float(rr["x"])-T[0], float(rr["y"])-T[1], float(rr["z"])-T[2]); loc = inv_rot(q, d)
        if cr not in base: base[cr] = loc
        dev = math.dist(loc, base[cr])
        if dev > 0.05 and cr not in first_slide: first_slide[cr] = (c, f["t"], dev, spd, yr, loc, base[cr], (rr["vx"], rr["vy"], rr["vz"]))
        if (loc[1] < base[cr][1] - 0.5 or math.hypot(loc[0]-base[cr][0], loc[2]-base[cr][2]) > 1.5) and cr not in first_leave:
            first_leave[cr] = (c, f["t"], spd, yr, loc)
    rows.append((c, f["t"], spd, yr, (r["tvx"], r["tvy"], r["tvz"]), (r["twx"], r["twy"], r["twz"])))
    prev = (c, T, q)
print("truck starts moving at counter", start_move)
for x in rows[::60]: print(" c", x[0], "t", round(x[1],1), "speed", x[2] and round(x[2],2), "yawrate", x[3] and round(x[3],1), "body v", x[4], "w", x[5])
print("first slide (>5 cm in truck frame):")
for cr, v in sorted(first_slide.items(), key=lambda kv: kv[1][0]): print(" ", cr, "c", v[0], "t", round(v[1],2), "dev", round(v[2],3), "truck speed", v[3] and round(v[3],2), "yaw rate", v[4] and round(v[4],1), "crate v", v[7])
print("first leave (>1.5 m or 0.5 m down):")
for cr, v in sorted(first_leave.items(), key=lambda kv: kv[1][0]): print(" ", cr, "c", v[0], "t", round(v[1],2), "truck speed", v[2] and round(v[2],2), "yaw rate", v[3] and round(v[3],1), "loc", tuple(round(a,2) for a in v[4]))
print("never slid:", [c for c in base if c not in first_slide]); print("never left:", [c for c in base if c not in first_leave])
