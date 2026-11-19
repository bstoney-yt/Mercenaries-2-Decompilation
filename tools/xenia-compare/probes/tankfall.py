# tankfall.py <out.txt> <tank guid hex> <seconds>: read-only, tank in the air. Per tank counter: dt, position, body v/w (store chain),
# every wheel/tread timer (0.4 = touching this step) and contact count. Once at the start (and after 20 s): every gravity action
# (vtable 0x820C8284) whose words point at the tank's P, wrappers or bodies, with its scale at +0x28 (expect 2.0 for tanks).
# Offline: fit the vertical acceleration over frames with no wheel touching (or a clear free-fall stretch).
import sys, struct, subprocess, time
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
sys.path.insert(0, "C:/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad")
import xmem, physchain as pc
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g)
tank, secs = int(sys.argv[2], 16), float(sys.argv[3]); t0 = time.time()
fmt = lambda t: "None" if t is None else "(" + ",".join(f"{x:.3f}" for x in t) + ")"
with open(sys.argv[1], "w") as fh:
    w = lambda s: (fh.write(s + "\n"), fh.flush())
    def actions(P, bl):
        own = {P: "P", **{a: "wrapper" for a, _ in bl}, **{b: "body" for _, b in bl}}
        for a in xmem.scan_word(g, 0x820C8284, 0x40000000, 0xC0000000, 256):
            b = g.read(a, 0x60) or b""
            ws = struct.unpack(f">{len(b)//4}I", b[: len(b)//4*4])
            hit = [(f"+{4*i:#x}", own[x]) for i, x in enumerate(ws) if x in own]
            if hit: w(f"gravity action {a:#x} scale+0x28 {struct.unpack_from('>f', b, 0x28)[0]:.3f} refs {hit}")
        w("action scan done")
    P = None; last = None; scan_at = 0
    while time.time() - t0 < secs:
        if not P:
            rec, P, how = pc.lookup(g, H, tank)
            if P: bl = pc.bodies(g, P)[3]; w(f"tank {tank:#x} P {P:#x} via {how} vt {pc.u32(g, P):#x} bodies {[(hex(a), hex(b)) for a, b in bl]}")
            else: time.sleep(0.2); continue
        if time.time() > scan_at: actions(P, bl); scan_at = time.time() + 20 if scan_at == 0 else 1e18
        sa = H.state_addr(tank); b = g.read(sa, 0x10) if sa else None
        if not b or len(b) < 0x10: continue
        c = struct.unpack_from(">I", b, 0xC)[0]
        if c == last: continue
        last = c
        dt = struct.unpack(">f", g.read(0x82B7C730, 4) or bytes(4))[0]
        v, wv = pc.body_state(g, bl[0][1]) if bl else (None, None)
        wh = " ".join(f"{t:.3f}/{k}" for t, k in pc.wheels(g, P)[1])
        w(f"c {c} t {time.time()-t0:.4f} dt {dt:.5f} pos {fmt(struct.unpack_from('>3f', b, 0))} v {fmt(v)} w {fmt(wv)} wheels {wh}")
