# bodyprobe.py <out.txt> <car x> <car z> <seconds>: read-only, first read of the #728 window.
# Tight loop on the car's state record only; on each counter change (one per game frame) log: counter, dt (0x82B7C730),
# position, quaternion, P = u32(entry+0x34) (car physics object), C = u32(P+0x158) (vehicle core), and a raw dump of P[0..0x200]
# and C[0..0x100] for the counters from 3 before the health hits 0 to 12 after (to find the rigid body pointer and its linear
# velocity offline: a float triple that matches the position deltas / dt), plus whether P / C are the same objects as before.
import sys, struct, subprocess, time, math
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
import xmem
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g)
cx, cz, secs = float(sys.argv[2]), float(sys.argv[3]), float(sys.argv[4])
hm = xmem.map_entries(g, 0x83656558); car = None; ca = None
for k, a in hm.items():
    sa = H.state_addr(k)
    if sa:
        p = struct.unpack(">3f", g.read(sa, 12))
        if math.hypot(p[0] - cx, p[2] - cz) < 1.0 and struct.unpack(">f", g.read(a, 4))[0] == 100.0:
            car, ca = k, a; break
def ptrs(P):
    """The first 16 heap pointers in P[0..0x200] (offset, target, target[0..0x100]): one of them should be the rigid body."""
    b = g.read(P, 0x200) if P else None; out = []
    for o in range(0, len(b or b"") - 3, 4):
        q = struct.unpack_from(">I", b, o)[0]
        if 0x40000000 <= q < 0xC0000000 and q & 3 == 0:
            d = g.read(q, 0x100)
            if d: out.append((o, q, d))
        if len(out) >= 16: break
    return out
entry = H.state_addr(car) if car else None
t0 = time.time(); last = None; ring = []; dead_c = None
with open(sys.argv[1], "w") as fh:
    w = lambda s: fh.write(s + "\n")
    w(f"car {car and hex(car)} state entry {entry and hex(entry)} health entry {ca and hex(ca)}")
    while time.time() - t0 < secs and entry:
        b = g.read(entry, 0x38)
        if not b or len(b) < 0x38: continue
        x, y, z, c, qx, qy, qz, qw = struct.unpack_from(">3fI4f", b, 0)
        if c == last: continue
        last = c
        P = struct.unpack_from(">I", b, 0x34)[0]
        C = struct.unpack(">I", g.read(P + 0x158, 4) or bytes(4))[0] if P else 0
        dtb = g.read(0x82B7C730, 4); hv = struct.unpack(">f", g.read(ca + 4, 4))[0] if ca else float("nan")
        row = (c, dtb and struct.unpack(">f", dtb)[0], (x, y, z), (qx, qy, qz, qw), P, C, hv,
               g.read(P, 0x200) if P else b"", (g.read(C, 0x100) if C else b"") or b"", ptrs(P))
        ring = (ring + [row])[-4:]
        if hv <= 0 and dead_c is None:
            dead_c = c
            for r in ring[:-1]:  # the 3 counters before the death
                w(f"counter {r[0]} dt {r[1]:.5f} pos {tuple(round(v,4) for v in r[2])} quat {tuple(round(v,5) for v in r[3])} P {r[4]:#x} C {r[5]:#x} health {r[6]:.2f}")
                w("  P " + r[7].hex()); w("  C " + r[8].hex()); [w(f"  *{o:#x} {q:#x} " + d.hex()) for o, q, d in r[9]]
        if dead_c is not None and c <= dead_c + 12:
            w(f"counter {c} dt {row[1]:.5f} pos {tuple(round(v,4) for v in row[2])} quat {tuple(round(v,5) for v in row[3])} P {P:#x} C {C:#x} health {hv:.2f}")
            w("  P " + row[7].hex()); w("  C " + row[8].hex()); [w(f"  *{o:#x} {q:#x} " + d.hex()) for o, q, d in row[9]]
        elif dead_c is not None and c > dead_c + 12:
            break
        fh.flush()
