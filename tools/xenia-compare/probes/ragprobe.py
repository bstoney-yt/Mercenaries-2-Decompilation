# ragprobe.py <out.txt> <car x> <car z> <seconds>: read-only, #673 land case. Every hero counter change from her seating:
# her pos, health, stance/action/mode; her impulse handler (store 0x8365849C, vtable 0x820EBF50) +0x1D8 countdown,
# +0x1BC queued impulse (3 floats), ragdoll-live bit (handler+0x18 -> +0x324 & 0x20); the car's health.
import sys, struct, subprocess, time, math
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
import xmem
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g)
cx, cz, secs = float(sys.argv[2]), float(sys.argv[3]), float(sys.argv[4])
u32 = lambda a: struct.unpack(">I", g.read(a, 4) or bytes(4))[0]
f32 = lambda a: struct.unpack(">f", g.read(a, 4) or bytes(4))[0]
VT = 0x820EBF50
def handler(h):
    for get in (lambda: xmem.map_entries(g, 0x8365849C).get(h), lambda: H.state_addr(h, 0x8365849C)):
        try: a = get()
        except Exception: a = None
        if not a: continue
        if u32(a) == VT: return a, "entry"
        if u32(u32(a)) == VT: return u32(a), "ptr"
    return None, None
t0 = time.time()
with open(sys.argv[1], "w") as fh:
    w = lambda s: (fh.write(s + "\n"), fh.flush())
    car = ca = None
    while time.time() - t0 < secs and car is None:  # find the parked car
        for k, a in xmem.map_entries(g, 0x83656558).items():
            sa = H.state_addr(k)
            if sa:
                p = struct.unpack(">3f", g.read(sa, 12))
                if math.hypot(p[0] - cx, p[2] - cz) < 1.5: car, ca = k, a; break
        time.sleep(0.2)
    hero = H.character(); hm = xmem.map_entries(g, 0x83656558).get(hero)
    hs = H.state_addr(hero); hd, how = handler(hero)
    w(f"car {car and hex(car)} hero {hero and hex(hero)} hero state {hs and hex(hs)} hero health {hm and hex(hm)} handler {hd and hex(hd)} via {how}")
    last = None; dead = None; idx = H.table_index(0x83657B94); n = 0
    while time.time() - t0 < secs and hero:
        hs = H.state_addr(hero)  # her record moves when she is seated / put out
        b = g.read(hs, 0x10) if hs else None
        if not b: continue
        c = struct.unpack_from(">I", b, 0xC)[0]
        if c == last: continue
        last = c; x, y, z = struct.unpack_from(">3f", b, 0)
        chp = f32(ca + 4) if ca else float("nan")
        if dead is None and chp <= 0: dead = c
        if hd is None or n % 10 == 0:
            hd2, how2 = handler(hero)
            if hd2 != hd: w(f"handler now {hd2 and hex(hd2)} via {how2}"); hd = hd2
        if n % 10 == 0:
            idx = H.table_index(0x83657B94); hp_ = xmem.map_entries(g, 0x83656558)
            hm, ca = hp_.get(hero, hm), hp_.get(car, ca)
        n += 1
        st = H.anim_state(hero, idx) or (0, 0, 0)
        if hd:
            cd = f32(hd + 0x1D8); imp = struct.unpack(">3f", g.read(hd + 0x1BC, 12) or bytes(12))
            rp = u32(hd + 0x18); live = (u32(rp + 0x324) if rp else 0)
            hs_ = f"cd {cd:.4f} imp {tuple(round(v,3) for v in imp)} r {rp:#x} live {live:#x} bit {int(bool(live & 0x20))}"
        else: hs_ = "no handler"
        rel = "" if dead is None else f" df {c - dead}"
        w(f"c {c}{rel} pos {x:.3f} {y:.3f} {z:.3f} hp {f32(hm + 4) if hm else float('nan'):.2f} car {chp:.1f} st {st[0]:#x} {st[1]:#x} {st[2]:#x} {hs_}")
        if dead is not None and c > dead + 150: break
