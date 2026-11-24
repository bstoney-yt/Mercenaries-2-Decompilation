# chopperprobe.py <out.txt> <bike x> <bike z> <seconds>: read-only, Chopper head-on (Jennifer). The bike = the vehicle record
# nearest (x, z) with car-class physics. Per bike counter: dt, bike pos/quat, bike body v/w (store chain), wheel touches,
# her position and stance/action/mode, her ragdoll actor bodies' v (pool 0x8365726C or controller+0x18) once FULL.
import sys, struct, subprocess, time, math
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
sys.path.insert(0, "C:/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad")
import xmem, physchain as pc
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g)
bx, bz, secs = float(sys.argv[2]), float(sys.argv[3]), float(sys.argv[4]); t0 = time.time()
fmt = lambda t: "None" if not t else "(" + ",".join(f"{x:.3f}" for x in t) + ")"
def find_bike():
    idx = H.table_index(H.STATES); store = xmem.map_entries(g, pc.STORE); best = None
    for h, a in idx.items():
        b = g.read(a, 12)
        if not b or len(b) < 12: continue
        x, y, z = struct.unpack(">3f", b); d = math.hypot(x - bx, z - bz)
        r = store.get(h); P = pc.u32(g, r) if r else 0
        if d < 3 and P and pc.u32(g, P) == pc.CAR_VT and (best is None or d < best[0]): best = (d, h, P)
    return best
with open(sys.argv[1], "w") as fh:
    w = lambda s: (fh.write(s + "\n"), fh.flush())
    best = None
    while best is None and time.time() - t0 < secs: best = find_bike(); time.sleep(0.2)
    if not best: w("no bike"); sys.exit()
    _, bike, P = best; bl = pc.bodies(g, P)[3]; hero = H.character()
    w(f"bike {bike:#x} P {P:#x} bodies {[(hex(a), hex(b)) for a, b in bl]} hero {hero and hex(hero)}")
    last = None; idx = H.table_index(0x83657B94); n = 0
    while time.time() - t0 < secs:
        sa = H.state_addr(bike); b = g.read(sa, 0x20) if sa else None
        if not b or len(b) < 0x20: continue
        c = struct.unpack_from(">I", b, 0xC)[0]
        if c == last: continue
        last = c; n += 1
        if n % 15 == 0: idx = H.table_index(0x83657B94)
        dt = struct.unpack(">f", g.read(0x82B7C730, 4) or bytes(4))[0]
        v, wv = pc.body_state(g, bl[0][1]) if bl else (None, None)
        wh = " ".join(f"{t:.2f}/{k}" for t, k in pc.wheels(g, P)[1])
        hs = H.state_addr(hero); hb = g.read(hs, 12) if hs else None
        st = H.anim_state(hero, idx) or (0, 0, 0)
        rag = ""
        if st[2] == 0x4294EBA2:
            e = xmem.map_entries(g, 0x8365726C).get(hero); act = None
            for a in ((e, pc.u32(g, e)) if e else ()):
                if pc.ok_ptr(a) and 0 < pc.u32(g, a + 0x50) <= 32: act = a; break
            if act:
                lst, nb = pc.u32(g, act + 0x4C), pc.u32(g, act + 0x50)
                vs = [pc.body_state(g, pc.u32(g, pc.u32(g, lst + 4 * i) + 0x38))[0] for i in range(min(nb, 12))]
                rag = f" rag {act:#x} v0 {fmt(vs[0] if vs else None)} vmean {fmt(tuple(sum(q[k] for q in vs if q) / max(1, sum(1 for q in vs if q)) for k in range(3)))}"
        w(f"c {c} t {time.time()-t0:.4f} dt {dt:.5f} bike {fmt(struct.unpack_from('>3f', b, 0))} q {fmt(struct.unpack_from('>4f', b, 0x10))} v {fmt(v)} w {fmt(wv)} wheels {wh} her {fmt(struct.unpack('>3f', hb) if hb else None)} st {st[0]:#x} {st[1]:#x} {st[2]:#x}{rag}")
