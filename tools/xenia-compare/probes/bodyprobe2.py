# bodyprobe2.py <out.txt> <car x> <car z> <seconds>: read-only, #728 first long-window item. Car found by its parked spot (health 100
# within 1 m). Per car counter (tight loop): dt, position, every body's linear (B+0x1A0) and angular (B+0x1B0) velocity, each wheel's
# timer (0.4 = touching this step) and contact count; full rows from 5 counters before the health reaches 0 to 30 after.
import sys, struct, subprocess, time, math
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
sys.path.insert(0, r"C:\Users\dev\AppData\Local\Temp\claude\b0147d3e513e77ed\61b02c61-65f9-431d-8481-4f2b8f31b793\scratchpad")
import xmem, physchain as pc
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g)
cx, cz, secs = float(sys.argv[2]), float(sys.argv[3]), float(sys.argv[4])
car = ca = None
for k, a in xmem.map_entries(g, 0x83656558).items():
    sa = H.state_addr(k)
    if sa:
        p = struct.unpack(">3f", g.read(sa, 12))
        if math.hypot(p[0] - cx, p[2] - cz) < 1.0 and struct.unpack(">f", g.read(a, 4))[0] == 100.0: car, ca = k, a; break
t0 = time.time()
fmt = lambda t: "None" if t is None else "(" + ",".join(f"{x:.3f}" for x in t) + ")"
with open(sys.argv[1], "w") as fh:
    w = lambda s: (fh.write(s + "\n"), fh.flush())
    entry = H.state_addr(car) if car else None
    rec, P, how = pc.lookup(g, H, car) if car else (None, None, None)
    vt = pc.u32(g, P) if P else 0
    lst, n, ind, bl = pc.bodies(g, P) if P else (0, 0, 0, [])
    C, ws = pc.wheels(g, P) if P else (0, [])
    w(f"car {car and hex(car)} state {entry and hex(entry)} store rec {rec and hex(rec)} via {how} P {P and hex(P)} vtable {vt:#x} "
      f"(want {pc.CAR_VT:#x}) bodies list {lst:#x} n {n} ind {ind:#x} {[(hex(a), hex(b)) for a, b in bl]} core {C:#x} wheels {len(ws)}")
    last = None; dead = None; ring = []; P0, B0 = P, [b for _, b in bl]
    while time.time() - t0 < secs and entry:
        b = g.read(entry, 0x10)
        if not b or len(b) < 0x10: continue
        c = struct.unpack_from(">I", b, 0xC)[0]
        if c == last: continue
        last = c
        hv = struct.unpack(">f", g.read(ca + 4, 4) or bytes(4))[0] if ca else float("nan")
        dt = struct.unpack(">f", g.read(0x82B7C730, 4) or bytes(4))[0]
        Pn = pc.u32(g, rec) if rec else 0
        bl = pc.bodies(g, Pn)[3] if Pn else []
        bs = " ".join(f"B{i} {B:#x} v{fmt(pc.body_state(g, B)[0])} w{fmt(pc.body_state(g, B)[1])}" for i, (_, B) in enumerate(bl) if pc.ok_ptr(B))
        wh = " ".join(f"{t:.3f}/{k}" for t, k in pc.wheels(g, Pn)[1]) if Pn else ""
        same = f"P {'same' if Pn == P0 else hex(Pn)} B {'same' if [x for _, x in bl] == B0 else 'NEW'}"
        row = f"c {c} dt {dt:.5f} pos {fmt(struct.unpack_from('>3f', b, 0))} hp {hv:.2f} {same} | {bs} | wheels {wh}"
        ring = (ring + [row])[-5:]
        if dead is None and hv <= 0:
            dead = c; [w(r) for r in ring[:-1]]
            fb = g.read(0x82DE8894, 16); w(f"death-exit floats 0x82DE8894.. {struct.unpack('>4f', fb) if fb else None} (image 10,0,10,0)")
        if dead is not None:
            w(row)
            if c > dead + 30: break
