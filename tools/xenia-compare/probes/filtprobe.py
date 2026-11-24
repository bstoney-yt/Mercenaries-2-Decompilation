# filtprobe.py <out.txt> <car x> <car z> <seconds>: read-only (Jennifer, rag-style Phoenix wreck). After the car's health reaches 0,
# at df 3, 30 and 90: every guid marked in the explosion actions (vtable 0x820C7A24, list u32(0x831334C8)+0x388C) and every state
# record within 20 m of her: store 0x836571D4 record -> P (vtable) -> each body: gamebody +0x34 -> Havok object, filter word +0x2C,
# decoded (layer bits 0-4, subsystem 5-9, no-collide 10-14, group 16-31) and whether FILTER[20] = 0x28C5BB6F has that layer's bit.
import sys, struct, subprocess, time, math
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
sys.path.insert(0, "C:/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad")
import xmem, physchain as pc
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g)
cx, cz, secs = float(sys.argv[2]), float(sys.argv[3]), float(sys.argv[4])
u32 = lambda a: pc.u32(g, a); F20 = 0x28C5BB6F
def dec(wd): return f"{wd:#010x} layer {wd & 31} subsys {(wd >> 5) & 31} nocollide {(wd >> 10) & 31} group {wd >> 16:#x} meets-ragdoll(F20 bit) {bool(F20 >> (wd & 31) & 1)}"
car = ca = None; t0 = time.time()
while car is None and time.time() - t0 < 60:
    for k, a in xmem.map_entries(g, 0x83656558).items():
        sa = H.state_addr(k)
        if sa:
            p = struct.unpack(">3f", g.read(sa, 12))
            if math.hypot(p[0] - cx, p[2] - cz) < 1.5 and struct.unpack(">f", g.read(a, 4))[0] == 100.0: car, ca = k, a; break
    if car is None: time.sleep(0.3)
def actions():
    mgr = u32(0x831334C8); raw = g.read(mgr + 0x388C, 0x40) or b""; acts = set()
    for o in range(0, len(raw) - 3, 4):
        q = struct.unpack_from(">I", raw, o)[0]
        if pc.ok_ptr(q):
            if u32(q) == 0x820C7A24: acts.add(q)
            for j in range(16):
                r = u32(q + 4 * j)
                if pc.ok_ptr(r) and u32(r) == 0x820C7A24: acts.add(r)
    return acts
def snapshot(w, tag, hero):
    marked = {}
    for a in actions():
        for i in range(min(u32(a + 0x218), 64)):
            e = g.read(a + 0x18 + 8 * i, 8)
            if e: marked[struct.unpack(">I", e[:4])[0]] = f"{struct.unpack('>I', e[4:])[0]:#x}"
    hs = H.state_addr(hero); hp = struct.unpack(">3f", g.read(hs, 12)) if hs else (0, 0, 0)
    idx = H.table_index(H.STATES); store = xmem.map_entries(g, pc.STORE); health = xmem.map_entries(g, 0x83656558)
    near = {}
    for h, a in idx.items():
        b = g.read(a, 12)
        if b and len(b) == 12:
            p = struct.unpack(">3f", b); d = math.dist(p, hp)
            if d < 20: near[h] = (d, p)
    w(f"== {tag}: her {hero:#x} at ({hp[0]:.2f},{hp[1]:.2f},{hp[2]:.2f}); marked {len(marked)}; records within 20 m {len(near)}")
    for h in sorted(set(marked) | set(near)):
        r = store.get(h); P = u32(r) if r else 0
        d, p = near.get(h, (None, None))
        hm = health.get(h); hv = struct.unpack(">2f", g.read(hm, 8)) if hm else None
        kind = "car" if h == car else ("her" if h == hero else ("spawned" if h >= 0x40000000 else "layer"))
        head = f"  {h:#x} [{kind}] mark {marked.get(h, '-')} dist {d and round(d, 2)} health {hv and tuple(round(x, 1) for x in hv)} P {P:#x} vt {u32(P) if P else 0:#x}"
        if not pc.ok_ptr(P): w(head + " (no store physics)"); continue
        lst, n, ind, bl = pc.bodies(g, P)
        ws = []
        for gb, B in bl[:4]:
            hk = u32(gb + 0x34)
            ws.append(dec(u32(hk + 0x2C)) if pc.ok_ptr(hk) else f"no hk {hk:#x}")
        w(head + f" n {n} ind {ind:#x} | " + " || ".join(ws))
with open(sys.argv[1], "w") as fh:
    w = lambda s: (fh.write(s + "\n"), fh.flush())
    hero = H.character(); w(f"car {car and hex(car)} hero {hero and hex(hero)}")
    entry = H.state_addr(car) if car else None; last = None; dead = None; done = set()
    while time.time() - t0 < secs and entry:
        if ca and struct.unpack(">f", g.read(ca + 4, 4) or bytes(4))[0] > 0 and dead is None:
            time.sleep(0.01); continue
        if dead is None: dead = time.time(); w("car health 0")
        el = time.time() - dead
        for k, at in ((3, 0.1), (30, 1.0), (90, 3.0)):
            if k not in done and el >= at:
                done.add(k); snapshot(w, f"df~{k} (+{el:.2f} s)", hero)
        if len(done) == 3: break
        time.sleep(0.01)
