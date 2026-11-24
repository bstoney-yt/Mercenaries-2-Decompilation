# bodyprobe3.py <out.txt> <car x> <car z> <seconds>: read-only, #728 phys4 (Jennifer). Car found by its parked spot (100 health).
# Per car counter from 3 before health 0 to 12 after: position; OLD body (kept after the swap) and NEW body: B+0x1A0 v, B+0x1B0 w,
# hk = gamebody+0x34 (Havok rigid body: +0x10 shape, +0x2C filter); P+0x38 byte (rebuild queued), P+0x48 record (+4 linear,
# +0x10 angular floats, and raw 0x20); each body's shape vtable + raw 0x80 and the first 6 pointers in it (child shapes) raw 0x40
# (box half-extents / AABB worked out offline); area-damage entries (pool 0x8365650C) with transforms (pool 0x8365808C).
import sys, struct, subprocess, time, math
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
sys.path.insert(0, "C:/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad")
import xmem, physchain as pc
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g)
cx, cz, secs = float(sys.argv[2]), float(sys.argv[3]), float(sys.argv[4])
u32 = lambda a: pc.u32(g, a)
f3 = lambda a: pc.f3(g, a)
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
def shape_dump(w, tag, gb):
    hk = u32(gb + 0x34); B = u32(gb + 0x38)
    sh = u32(hk + 0x10) if pc.ok_ptr(hk) else 0
    w(f"  {tag}: gamebody {gb:#x} hk(+0x34) {hk:#x} B(+0x38) {B:#x} same {hk == B} filter {u32(hk + 0x2C):#010x} shape {sh:#x} vt {u32(sh):#x}")
    if pc.ok_ptr(sh):
        raw = g.read(sh, 0x80) or b""; w(f"    shape raw {raw.hex()}")
        seen = 0
        for o in range(0, len(raw) - 3, 4):
            q = struct.unpack_from(">I", raw, o)[0]
            if pc.ok_ptr(q) and q != sh and seen < 6:
                seen += 1; w(f"    shape+{o:#x} -> {q:#x} vt {u32(q):#x} raw {(g.read(q, 0x40) or b'').hex()}")
    if pc.ok_ptr(B): w(f"    B[0xC0:0xE0] {(g.read(B + 0xC0, 0x20) or b'').hex()}")
with open(sys.argv[1], "w") as fh:
    w = lambda s: (fh.write(s + "\n"), fh.flush())
    entry = H.state_addr(car) if car else None
    rec, P, how = pc.lookup(g, H, car) if car else (None, None, None)
    w(f"car {car and hex(car)} P {P and hex(P)} via {how} vt {P and hex(u32(P))}")
    if not P: sys.exit()
    old = pc.bodies(g, P)[3]; old_gb, old_B = old[0] if old else (0, 0)
    shape_dump(w, "old body at start", old_gb)
    last = None; dead = None; ring = []; seen = set(xmem.map_entries(g, 0x8365650C)); new_done = False
    while time.time() - t0 < secs and entry:
        b = g.read(entry, 0x20)
        if not b or len(b) < 0x20: continue
        c = struct.unpack_from(">I", b, 0xC)[0]
        if c == last: continue
        last = c
        hv = struct.unpack(">f", g.read(ca + 4, 4) or bytes(4))[0] if ca else float("nan")
        cur = pc.bodies(g, P)[3]; gb, B = cur[0] if cur else (0, 0)
        rec48 = u32(P + 0x48); r38 = (g.read(P + 0x38, 1) or b"\xff")[0]
        row = (f"c {c} dt {struct.unpack('>f', g.read(0x82B7C730, 4) or bytes(4))[0]:.5f} pos {fmt(struct.unpack_from('>3f', b, 0))} hp {hv:.2f} "
               f"| OLD B {old_B:#x} v{fmt(f3(old_B + 0x1A0))} w{fmt(f3(old_B + 0x1B0))} | CUR B {B:#x} {'NEW' if B != old_B else 'same'} "
               f"v{fmt(f3(B + 0x1A0))} w{fmt(f3(B + 0x1B0))} | P+0x38 {r38} P+0x48 {rec48:#x}"
               + (f" lin{fmt(f3(rec48 + 4))} ang{fmt(f3(rec48 + 0x10))} raw {(g.read(rec48, 0x20) or b'').hex()}" if pc.ok_ptr(rec48) else ""))
        ring = (ring + [row])[-3:]
        if dead is None and hv <= 0:
            dead = c; [w(r) for r in ring[:-1]]
        if dead is not None:
            w(row)
            if B != old_B and not new_done: new_done = True; shape_dump(w, "NEW body", gb)
            for k, a in xmem.map_entries(g, 0x8365650C).items():
                if k in seen: continue
                seen.add(k); e = g.read(a, 0x40) or bytes(0x40)
                tr = xmem.map_entries(g, 0x8365808C).get(k); tp = struct.unpack(">3f", g.read(tr, 12)) if tr else None
                cp = struct.unpack_from(">3f", b, 0)
                w(f"  area-damage {k:#x} at c {c}: template {struct.unpack_from('>I', e, 0x30)[0]:#x} pos {fmt(tp)} rel car record {fmt(tuple(tp[i] - cp[i] for i in range(3)) if tp else None)}")
            if c > dead + 12: break
