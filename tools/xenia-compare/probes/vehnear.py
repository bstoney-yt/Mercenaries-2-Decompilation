# vehnear.py <x> <z> <r>: health-pool records within r of (x, z) with max health >= 100 that are not people
import sys, struct, subprocess, math
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
import xmem
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g)
x, z, R = map(float, sys.argv[1:4]); aidx = H.table_index(0x83657B94)
for k, a in xmem.map_entries(g, 0x83656558).items():
    sa = H.state_addr(k)
    if not sa: continue
    p = struct.unpack(">3f", g.read(sa, 12)); d = math.hypot(p[0]-x, p[2]-z)
    if d > R: continue
    mx, cur = struct.unpack(">2f", g.read(a, 8))
    person = aidx.get(k) is not None
    print(hex(k), [round(c,1) for c in p], f"{cur:.0f}/{mx:.0f}", round(d,1), "person" if person else "")
