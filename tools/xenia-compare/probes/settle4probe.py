# settle4probe.py <outdir> <seconds>: read-only (OilCon020, El Grande 0xFA056 + crate 0x11D397, also 0x11D3A0). From the truck's first
# record, every 0.1 s for 90 s: frame clock mgr+0x23D4, and per object: body B (store 0x836571D4 chain) motion type byte B+0xD8,
# v B+0x1A0, P+0x18 (u32), scene object (pool 0x8365808C entry) +0x18 (u32; &0xC: 0 awake, 4 hibernating, 8 hibernated, as asked);
# on change only: 0x836512A4 (0x20 bytes, the handler bitmask) and the tables 0x82F0D470 / 0x82F0D270 (0x80 bytes each).
import sys, struct, subprocess, time, os
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
sys.path.insert(0, "C:/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad")
import xmem, physchain as pc
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g); u32 = lambda a: pc.u32(g, a)
od, secs = sys.argv[1], float(sys.argv[2]); os.makedirs(od, exist_ok=True); t0 = time.time()
IDS = (0xFA056, 0x11D397, 0x11D3A0)
fh = open(os.path.join(od, "settle4.txt"), "w"); w = lambda s: (fh.write(s + "\n"), fh.flush())
while time.time() - t0 < secs and not H.state_addr(0xFA056): time.sleep(0.02)
t1 = time.time(); mgr = u32(0x831334C8); w(f"truck first record at t {t1-t0:.2f}")
info = {}; last_glob = None
while time.time() - t1 < 90:
    glob = (g.read(0x836512A4, 0x20) or b"", g.read(0x82F0D470, 0x80) or b"", g.read(0x82F0D270, 0x80) or b"")
    if glob != last_glob:
        last_glob = glob; w(f"GLOBALS t {time.time()-t1:.2f} 0x836512A4 {glob[0].hex()} | 0x82F0D470 {glob[1].hex()} | 0x82F0D270 {glob[2].hex()}")
    row = f"t {time.time()-t1:.2f} step {u32(mgr + 0x23D4)}"
    so = xmem.map_entries(g, 0x8365808C)
    for gd in IDS:
        if gd not in info or not pc.ok_ptr(info[gd][1]):
            rec, P, how = pc.lookup(g, H, gd); bl = pc.bodies(g, P)[3] if P else []
            info[gd] = (P, bl[0][1] if bl else 0)
        P, B = info[gd]; s = so.get(gd)
        mt = g.read(B + 0xD8, 1) if pc.ok_ptr(B) else None; v = pc.f3(g, B + 0x1A0) if pc.ok_ptr(B) else None
        sw = u32(s + 0x18) if s else None
        row += (f" | {gd:#x} P {P or 0:#x} P+0x18 {u32(P + 0x18) if P else 0:#010x} B {B:#x} motion {mt[0] if mt else None} "
                f"v {tuple(round(x, 4) for x in v) if v else None} SO {s or 0:#x} SO+0x18 {sw if sw is None else f'{sw:#010x}'} state&0xC {None if sw is None else (sw >> 16 if False else sw) & 0xC}")
    w(row); time.sleep(0.1)
