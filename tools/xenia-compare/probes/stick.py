# stick.py <poke.txt> <x> <z> <tol m>: exit 0 if the last logged record position in tpoke's log is within tol of (x, z).
import sys, re, math
last = None
for l in open(sys.argv[1]):
    m = re.search(r"record \(([-\d.]+), ([-\d.]+), ([-\d.]+)\)", l)
    if m: last = (float(m[1]), float(m[3]))
x, z, tol = map(float, sys.argv[2:5])
d = math.hypot(last[0] - x, last[1] - z) if last else 1e9
print("last record", last, "dist", round(d, 2)); sys.exit(0 if d <= tol else 1)
