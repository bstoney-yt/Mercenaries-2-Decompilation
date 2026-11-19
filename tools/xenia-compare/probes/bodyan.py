import re, struct, sys
rows = []; cur = None
for l in open(sys.argv[1]):
    m = re.match(r"counter (\d+) dt ([\d.]+) pos \(([^)]*)\)", l)
    if m:
        cur = {"c": int(m[1]), "dt": float(m[2]), "pos": tuple(map(float, m[3].split(","))), "blk": {}}; rows.append(cur); continue
    m = re.match(r"  (P|C) ([0-9a-f]*)$", l)
    if m and cur: cur["blk"][m[1]] = bytes.fromhex(m[2]); continue
    m = re.match(r"  \*(0x[0-9a-f]+) (0x[0-9a-f]+) ([0-9a-f]*)$", l)
    if m and cur: cur["blk"][f"*{m[1]}"] = bytes.fromhex(m[3]); cur.setdefault("tgt", {})[m[1]] = m[2]
# velocity estimates: central difference using counters at 1/30 s
vel = {}
for a, b in zip(rows, rows[1:]):
    n = b["c"] - a["c"]
    vel[b["c"]] = tuple((q - p) * 30 / n for p, q in zip(a["pos"], b["pos"]))
for r in rows: print(r["c"], r["pos"], vel.get(r["c"]), r.get("tgt") and sorted(set(r["tgt"].items()))[:0])
# score every block/offset triple vs the backward-difference velocity over counters >= death+1
cands = {}
for r in rows:
    v = vel.get(r["c"]); 
    if not v or abs(v[1]) < 1: continue
    for k, b in r["blk"].items():
        for o in range(0, len(b) - 11, 4):
            t = struct.unpack_from(">3f", b, o)
            err = sum(abs(x - y) for x, y in zip(t, v)) / (sum(abs(y) for y in v))
            cands.setdefault((k, o), []).append(err)
best = sorted(((sum(e) / len(e), len(e), k) for k, e in cands.items() if len(e) >= 5), key=lambda x: x[0])[:12]
for s, n, (k, o) in best: print(f"{k} +{o:#x}: mean rel err {s:.3f} over {n}")
# pointer stability
print("targets first/last:", rows[0].get("tgt"), rows[-1].get("tgt") == rows[0].get("tgt"))
