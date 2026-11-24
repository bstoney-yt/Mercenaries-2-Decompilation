# massprobe.py (phys7; dmgprobe plus mass reads) <out.txt> <car x> <car z> <seconds>: read-only, #728. Car found by its parked spot (100 health). Per car counter from
# 3 before health 0 to 4 after: the car's RuntimeLastDamageApplied record (store 0x83656E7C keyed by the car guid; raw 0x40 and
# floats [0..15]: dir [0..2], hit point [3..5], kind [6]), old and new body B (store chain): position B+0x110 (as the ragdoll
# bodies showed), raw B+0x100..0x140, v B+0x1A0, w B+0x1B0; the car's state record position and quaternion.
import sys, struct, subprocess, time, math
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
sys.path.insert(0, "C:/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad")
import xmem, physchain as pc
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g)
cx, cz, secs = float(sys.argv[2]), float(sys.argv[3]), float(sys.argv[4])
u32 = lambda a: pc.u32(g, a); fmt = lambda t: "None" if not t else "(" + ",".join(f"{x:.4f}" for x in t) + ")"
car = ca = None; t0 = time.time()
while car is None and time.time() - t0 < 60:
    for k, a in xmem.map_entries(g, 0x83656558).items():
        sa = H.state_addr(k)
        if sa:
            p = struct.unpack(">3f", g.read(sa, 12))
            if math.hypot(p[0] - cx, p[2] - cz) < 1.5 and struct.unpack(">f", g.read(a, 4))[0] == 100.0: car, ca = k, a; break
    if car is None: time.sleep(0.3)
t0 = time.time()
def dmg_rec():
    for how, get in (("keyed", lambda: xmem.map_entries(g, 0x83656E7C).get(car)), ("table", lambda: H.state_addr(car, 0x83656E7C))):
        try: e = get()
        except Exception: e = None
        if e: return e, how
    return None, None
def body(B):
    if not pc.ok_ptr(B): return "none"
    m = g.read(B + 0xD0, 4) or bytes(4); inert = struct.unpack(">4f", g.read(B + 0x190, 16) or bytes(16))
    # 0x8262C120(B+0xD0) = 1/w of the vector at (B+0xD0)+0xC0 (lvx128 + vspltw + lfs 1.0, read from its instruction words):
    # Havok motion at B+0xD0, inertiaAndMassInv at motion+0xC0 = B+0x190, its w = inverse mass
    mass_s = f"B+0xD0 raw {m.hex()} f {struct.unpack('>f', m)[0]:.4g} | inertiaAndMassInv B+0x190 {fmt(inert)} -> mass 1/w {(1 / inert[3]) if inert[3] else float('inf'):.3f}"
    return f"{mass_s} | B {B:#x} pos+0x110 {fmt(pc.f3(g, B + 0x110))} v {fmt(pc.f3(g, B + 0x1A0))} w {fmt(pc.f3(g, B + 0x1B0))} raw100 {(g.read(B + 0x100, 0x40) or b'').hex()}"
with open(sys.argv[1], "w") as fh:
    w = lambda s: (fh.write(s + "\n"), fh.flush())
    entry = H.state_addr(car) if car else None
    rec, P, how = pc.lookup(g, H, car) if car else (None, None, None)
    old = pc.bodies(g, P)[3] if P else []; oldB = old[0][1] if old else 0
    w(f"car {car and hex(car)} P {P and hex(P)} old B {oldB:#x}")
    last = None; dead = None; ring = []
    while time.time() - t0 < secs and entry:
        b = g.read(entry, 0x20)
        if not b or len(b) < 0x20: continue
        c = struct.unpack_from(">I", b, 0xC)[0]
        if c == last: continue
        last = c
        hv = struct.unpack(">f", g.read(ca + 4, 4) or bytes(4))[0] if ca else float("nan")
        e, eh = dmg_rec(); raw = g.read(e, 0x40) if e else None
        fl = struct.unpack(">16f", raw) if raw and len(raw) == 0x40 else None
        cur = pc.bodies(g, P)[3] if P else []; B = cur[0][1] if cur else 0
        row = [f"c {c} hp {hv:.2f} record pos {fmt(struct.unpack_from('>3f', b, 0))} quat {fmt(struct.unpack_from('>4f', b, 0x10))}",
               f"  dmg rec {e and hex(e)} via {eh}: dir {fmt(fl[0:3]) if fl else None} hit {fmt(fl[3:6]) if fl else None} kind {fl and fl[6]} (as u32 {hex(struct.unpack_from('>I', raw, 24)[0]) if raw and len(raw) >= 28 else None}) floats {fmt(fl) if fl else None} raw {raw and raw.hex()}",
               f"  world u32(B+8) {u32(B + 8) if B else 0:#x} world+0x84 {u32(u32(B + 8) + 0x84) if B and u32(B + 8) else 0:#x} (0x8262AB60 defers the impulse as op 0x1A when non-zero; sampled between frames only)",
               f"  P+0x3C {g.read(P + 0x3C, 4).hex() if P else None} P+0x50 {g.read(P + 0x50, 4).hex() if P else None} (gate for the last-damage kick)",
               f"  OLD {body(oldB)}", f"  CUR {'NEW' if B != oldB else 'same'} {body(B)}",
               f"  rand scales 0x8380EB38/3C {fmt(struct.unpack('>2f', g.read(0x8380EB38, 8) or bytes(8)))} RNG 0x82B5E588 raw {(g.read(0x82B5E588, 0x20) or b'').hex()}"]
        ring = (ring + [row])[-4:]
        if dead is None and hv <= 0:
            dead = c; [w(l) for r in ring[:-1] for l in r]
        if dead is not None:
            [w(l) for l in row]
            if c > dead + 4: break
