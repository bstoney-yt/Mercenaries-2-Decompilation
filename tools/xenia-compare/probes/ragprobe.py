# ragprobe.py <out.txt> <car x> <car z> <seconds>: read-only, #673 land case. Every hero counter change from her seating:
# her pos, health, stance/action/mode; her impulse handler (store 0x8365849C, vtable 0x820EBF50) +0x1D8 countdown,
# +0x1BC queued impulse (3 floats), ragdoll-live bit (handler+0x18 -> +0x324 & 0x20); the car's health.
import sys, struct, subprocess, time, math
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
import xmem
sys.path.insert(0, "C:/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad")
import physchain as pc
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
def decode(wd):
    return f"{wd:#010x} layer {wd & 31} subsys {(wd >> 5) & 31} nocollide {(wd >> 10) & 31} group {wd >> 16:#x}"
def layer20(wd): return "CONFIRMS layer 20" if wd & 31 == 20 else f"CONTRADICTS layer 20 (layer {wd & 31})"
def actor_from_pool(hero):
    """Primary path (ragdoll thread): pool 0x8365726C (PhysicsActorRagdoll) keyed by her entity. Pool class not checked offline:
    try the keyed pool then the state-table shape; the entry is the actor itself or holds a pointer to it."""
    for how, get in (("keyed", lambda: xmem.map_entries(g, 0x8365726C).get(hero)), ("table", lambda: H.state_addr(hero, 0x8365726C))):
        try: e = get()
        except Exception: e = None
        if not e: continue
        for kind, a in (("entry", e), ("*entry", u32(e))):
            if pc.ok_ptr(a) and 0 < u32(a + 0x50) <= 32 and pc.ok_ptr(u32(a + 0x4C)): return a, f"{how}/{kind}", e
        return None, f"{how}: entry {e:#x} holds no body array", e
    return None, "no entry", None
def filt(gb):
    """game body -> +0x34 Havok object -> +0x2C collision filter word (coordinator's chain)."""
    hk = u32(gb + 0x34)
    if not pc.ok_ptr(hk): return f"no hk ({hk:#x})"
    wd = u32(hk + 0x2C); return f"{decode(wd)} {layer20(wd)}"
def actor_bodies(actor):
    """Body array of the ragdoll actor (0x82428768: actor = controller+0x18; array +0x4C, count +0x50, pointer list when +0x54 != 0)."""
    lst, n, ind = u32(actor + 0x4C), u32(actor + 0x50), u32(actor + 0x54)
    if not pc.ok_ptr(lst) or not 0 < n <= 32: return lst, n, ind, []
    return lst, n, ind, [u32(lst + 4 * i) for i in range(n)]
def filters_block(w, hero, car, actor_ctl):
    actor, how, e = actor_from_pool(hero)
    w(f"FILTERS actor via pool 0x8365726C: {actor and hex(actor)} ({how}, entry {e and hex(e)}); via controller+0x18: {actor_ctl:#x}; "
      f"{'AGREE' if actor == actor_ctl else 'DISAGREE'}")
    if not actor: actor = actor_ctl
    lst, n, ind, gbs = actor_bodies(actor)
    w(f"FILTERS ragdoll actor {actor:#x} array {lst:#x} n {n} ind {ind:#x} (when ind == 0 the element layout is unconfirmed; listed as pointers)")
    for i, gb in enumerate(gbs):
        w(f"  rag body {i} {gb:#x} hkbody(+0x38) {u32(gb + 0x38):#x} filter {filt(gb) if pc.ok_ptr(gb) else '-'}")
    for tag, guid in (("car", car), ("her store record", hero)):
        rec, P, how = pc.lookup(g, H, guid)
        bl = pc.bodies(g, P)[3] if P else []
        for i, (gb, B) in enumerate(bl):
            w(f"  {tag} body {i} {gb:#x} P vt {u32(P):#x} filter {filt(gb)}")
        if not bl: w(f"  {tag}: no bodies (P {P and hex(P)})")
    # props: the first 3 layer objects near her with non-car physics in the store
    store = xmem.map_entries(g, pc.STORE); idx = H.table_index(H.STATES); hp_ = H.state_addr(hero)
    hb = struct.unpack(">3f", g.read(hp_, 12)) if hp_ else (0, 0, 0); n_p = 0
    for h, a in sorted(idx.items(), key=lambda kv: kv[0]):
        if h >= 0x40000000 or n_p >= 3: continue
        b = g.read(a, 12)
        if not b or math.hypot(struct.unpack(">3f", b)[0] - hb[0], struct.unpack(">3f", b)[2] - hb[2]) > 80: continue
        r = store.get(h); P = u32(r) if r else 0
        if not pc.ok_ptr(P) or u32(P) == pc.CAR_VT: continue
        bl = pc.bodies(g, P)[3]
        if bl: w(f"  prop {h:#x} P vt {u32(P):#x} body 0 filter {filt(bl[0][0])}"); n_p += 1
    w("  character proxy: no chain known (not read)")
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
        if dead is None and chp <= 0:
            dead = c; fb = g.read(0x82DE8894, 16)
            w(f"death-exit floats 0x82DE8894.. {struct.unpack('>4f', fb) if fb else None} (image 10,0,10,0)")
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
        if dead is not None and hd and not getattr(sys, "_filt_done", False):
            rp_ = u32(hd + 0x18)
            if rp_ and (g.read(rp_ + 0x324, 1) or bytes(1))[0] & 0x20:
                sys._filt_done = True
                w(f"controller {hd:#x} entity(+0x14) {u32(hd + 0x14):#x} {'= her' if u32(hd + 0x14) == hero else '!= her ' + hex(hero)}")
                filters_block(w, hero, car, rp_)
        if dead is not None and c - dead <= 12:  # her physics bodies through the store chain (car layout assumed)
            rec, Ph, how = pc.lookup(g, H, hero)
            if Ph:
                lst, nb, ind, bl = pc.bodies(g, Ph)
                bv = " ".join(f"B{i} v{tuple(round(x,2) for x in (pc.body_state(g,B)[0] or ()))}" for i, (_, B) in enumerate(bl) if pc.ok_ptr(B))
                w(f"  phys rec {rec:#x} via {how} P {Ph:#x} vt {pc.u32(g, Ph):#x} n {nb} ind {ind:#x} {bv}")
            else: w("  phys: no store record")
            cs = H.state_addr(car); cr = g.read(cs, 0x20) if cs else None
            cp = struct.unpack_from(">3f", cr, 0) if cr else None; cq = struct.unpack_from(">4f", cr, 0x10) if cr else None
            rc, Pc, _ = pc.lookup(g, H, car); cb = pc.bodies(g, Pc)[3] if Pc else []
            cv = [tuple(round(v, 2) for v in (pc.body_state(g, B)[0] or ())) for _, B in cb if pc.ok_ptr(B)]
            if cp:
                # her offset in the car's frame (rotate by the conjugate quaternion); box = approximate Phoenix half-extents (invented:
                # 0.95 wide, 0.75 up from the record, 2.25 long), no AABB read
                qx, qy, qz, qw = cq; d = (x - cp[0], y - cp[1], z - cp[2])
                ux, uy, uz = -qx, -qy, -qz
                tx = 2 * (uy * d[2] - uz * d[1]); ty = 2 * (uz * d[0] - ux * d[2]); tz = 2 * (ux * d[1] - uy * d[0])
                loc = (d[0] + qw * tx + (uy * tz - uz * ty), d[1] + qw * ty + (uz * tx - ux * tz), d[2] + qw * tz + (ux * ty - uy * tx))
                inbox = abs(loc[0]) <= 0.95 and -0.3 <= loc[1] <= 0.75 + 0.9 and abs(loc[2]) <= 2.25
                w(f"  car pos ({cp[0]:.3f},{cp[1]:.3f},{cp[2]:.3f}) quat {tuple(round(q,4) for q in cq)} body v {cv} her offset horiz {math.hypot(x-cp[0], z-cp[2]):.3f} up {y-cp[1]:+.3f} "
                  f"car-frame ({loc[0]:+.3f},{loc[1]:+.3f},{loc[2]:+.3f}) above roof {loc[1]-0.75:+.3f} in approx box (+0.9 m for her height) {inbox}")
            else: w(f"  car: no state record; body v {cv}")
        w(f"c {c}{rel} pos {x:.3f} {y:.3f} {z:.3f} hp {f32(hm + 4) if hm else float('nan'):.2f} car {chp:.1f} st {st[0]:#x} {st[1]:#x} {st[2]:#x} {hs_}")
        if dead is not None and c > dead + 150: break
