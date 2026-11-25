# settleprobe.py <out.txt> <seconds>: read-only (crates-settle-ask.md). Truck 0xFA056 and crates 0x11D397-0x11D3A0, 0x1225F2-0x1225FB.
# From the truck's first record: once per truck counter for 20 s, then one sample at +60 s. Per object (store 0x836571D4 -> P -> body B,
# inline/pointer list rule): B+0x110 position, rows B+0xE0/+0xF0/+0x100, v B+0x1A0; plus its state-record position.
import sys, struct, subprocess, time
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
sys.path.insert(0, "C:/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad")
import xmem, physchain as pc
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g); secs = float(sys.argv[2]); t0 = time.time()
TRUCK = 0xFA056; IDS = [TRUCK] + list(range(0x11D397, 0x11D3A1)) + list(range(0x1225F2, 0x1225FC))
f3 = lambda a: pc.f3(g, a); fmt = lambda t: "None" if not t else "(" + ",".join(f"{x:.4f}" for x in t) + ")"
bodyof = {}
def body(gd):
    if gd not in bodyof or not pc.ok_ptr(bodyof[gd]):
        rec, P, how = pc.lookup(g, H, gd); bl = pc.bodies(g, P)[3] if P else []
        bodyof[gd] = bl[0][1] if bl else 0
    return bodyof[gd]
def sample(fh, tag, c):
    hp = H.update(); fh.write(f"{tag} t {time.time()-t0:.3f} truck c {c} her {hp and tuple(round(x, 1) for x in hp)}\n")
    for gd in IDS:
        sa = H.state_addr(gd)
        if not sa: continue
        B = body(gd)
        rp = struct.unpack(">3f", g.read(sa, 12) or bytes(12))
        fh.write(f"  {gd:#x} rec {fmt(rp)} B {B:#x} pos {fmt(f3(B + 0x110) if B else None)} rE0 {fmt(f3(B + 0xE0) if B else None)} "
                 f"rF0 {fmt(f3(B + 0xF0) if B else None)} r100 {fmt(f3(B + 0x100) if B else None)} v {fmt(f3(B + 0x1A0) if B else None)}\n")
    fh.flush()
with open(sys.argv[1], "w") as fh:
    ta = None
    while time.time() - t0 < secs:
        ta = H.state_addr(TRUCK)
        if ta: break
        time.sleep(0.02)
    if not ta: fh.write("truck never appeared\n"); sys.exit()
    t1 = time.time(); last = None
    present = [hex(gd) for gd in IDS if H.state_addr(gd)]
    fh.write(f"first truck record at t {t1-t0:.2f}; ids with records now: {present}\n")
    while time.time() - t1 < 20:
        c = pc.u32(g, ta + 0xC)
        if c != last: last = c; sample(fh, "s", c)
    while time.time() - t1 < 60: time.sleep(0.2)
    sample(fh, "s60", pc.u32(g, ta + 0xC))
    fh.write("ids missing a record at +60 s: " + str([hex(gd) for gd in IDS if not H.state_addr(gd)]) + "\n")
