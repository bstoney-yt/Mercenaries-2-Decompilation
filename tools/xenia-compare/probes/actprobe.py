# actprobe.py <out.txt> <car x> <car z> <seconds>: read-only, #728 phys5 (Jennifer). The explosion actions (vtable 0x820C7A24) on
# the car's death counter and the counters after: found through the manager list (u32(0x831334C8) + 0x388C; walked as pointers, one
# and two levels deep) and, once after the death, a memory scan for the vtable as a cross-check. Per action: centre +0x21C, radius
# +0x228, MaxForce +0x22C, MinForceFalloff +0x230, life +0x238 (raw floats +0x21C..+0x240), mark count +0x218, marks at +0x18
# (8-byte entries: guid, mark). Also the car's new body v (store chain) per counter.
import sys, struct, subprocess, time, math
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
sys.path.insert(0, "C:/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad")
import xmem, physchain as pc
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g)
cx, cz, secs = float(sys.argv[2]), float(sys.argv[3]), float(sys.argv[4])
u32 = lambda a: pc.u32(g, a); VT = 0x820C7A24
fmt = lambda t: "None" if not t else "(" + ",".join(f"{x:.3f}" for x in t) + ")"
car = ca = None; t0 = time.time()
while car is None and time.time() - t0 < 60:
    for k, a in xmem.map_entries(g, 0x83656558).items():
        sa = H.state_addr(k)
        if sa:
            p = struct.unpack(">3f", g.read(sa, 12))
            if math.hypot(p[0] - cx, p[2] - cz) < 1.5 and struct.unpack(">f", g.read(a, 4))[0] == 100.0: car, ca = k, a; break
    if car is None: time.sleep(0.3)
t0 = time.time()
def from_list():
    mgr = u32(0x831334C8); base = mgr + 0x388C; found = set()
    raw = g.read(base, 0x40) or b""
    for o in range(0, len(raw) - 3, 4):
        q = struct.unpack_from(">I", raw, o)[0]
        if not pc.ok_ptr(q): continue
        if u32(q) == VT: found.add(q); continue
        for j in range(16):  # q may be an array of action pointers
            r = u32(q + 4 * j)
            if pc.ok_ptr(r) and u32(r) == VT: found.add(r)
    return mgr, raw, found
def dump(w, a, tag):
    fl = struct.unpack(">9f", g.read(a + 0x21C, 36) or bytes(36))
    n = u32(a + 0x218); marks = []
    for i in range(min(n, 32)):
        e = g.read(a + 0x18 + 8 * i, 8)
        if e: marks.append(f"{struct.unpack('>I', e[:4])[0]:#x}:{struct.unpack('>I', e[4:])[0]:#x}")
    w(f"  {tag} action {a:#x}: centre ({fl[0]:.3f},{fl[1]:.3f},{fl[2]:.3f}) radius {fl[3]:.3f} MaxForce {fl[4]:.3f} MinForceFalloff {fl[5]:.3f} "
      f"+0x234 {fl[6]:.3f} life {fl[7]:.3f} +0x23C {fl[8]:.3f} | marks n {n} {marks} {'CAR MARKED x' + str(sum(1 for m in marks if m.startswith(hex(car) + ':'))) if car else ''}")
with open(sys.argv[1], "w") as fh:
    w = lambda s: (fh.write(s + "\n"), fh.flush())
    entry = H.state_addr(car) if car else None
    rec, P, how = pc.lookup(g, H, car) if car else (None, None, None)
    mgr, raw, f0 = from_list()
    w(f"car {car and hex(car)} P {P and hex(P)} manager {mgr:#x} list+0x388C raw {raw.hex()} actions now {[hex(x) for x in f0]}")
    last = None; dead = None; known = set(f0); scanned = False
    while time.time() - t0 < secs and entry:
        b = g.read(entry, 0x20)
        if not b or len(b) < 0x20: continue
        c = struct.unpack_from(">I", b, 0xC)[0]
        if c == last: continue
        last = c
        hv = struct.unpack(">f", g.read(ca + 4, 4) or bytes(4))[0] if ca else float("nan")
        if dead is None and hv <= 0: dead = c
        if dead is None: continue
        _, raw, acts = from_list(); known |= acts
        bl = pc.bodies(g, P)[3] if P else []
        v = pc.body_state(g, bl[0][1]) if bl else (None, None)
        w(f"c {c} df {c - dead} pos {fmt(struct.unpack_from('>3f', b, 0))} body v{fmt(v[0])} w{fmt(v[1])} list raw {raw[:0x20].hex()} actions {[hex(x) for x in sorted(acts)]}")
        for a in sorted(known): dump(w, a, "list" if a in acts else "known")
        if c - dead >= 2 and not scanned:
            scanned = True
            hits = xmem.scan_word(g, VT, 0x40000000, 0xC0000000, 64)
            w(f"  scan for vtable {VT:#x}: {[hex(x) for x in hits]}")
            for a in hits:
                if a not in known: dump(w, a, "scan")
            known |= set(hits)
        if c - dead > 6: break
