# dockprobe.py <out.txt> <seconds> <guid hex>...: read-only (Jennifer). For each guid from first sight, every 0.25 s: state-record
# position, yaw, up vector, body velocity (store 0x836571D4 chain, inline/pointer list rule); once at first sight: P vtable and each
# body's collision filter word (game body +0x34 -> +0x2C), raw and decoded (layer bits 0-4, subsystem 5-9, no-collide 10-14, group 16-31).
import sys, struct, subprocess, time, math
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
sys.path.insert(0, "C:/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad")
import xmem, physchain as pc
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g); secs = float(sys.argv[2]); guids = [int(x, 16) for x in sys.argv[3:]]; t0 = time.time()
dec = lambda wd: f"{wd:#010x} layer {wd & 31} subsys {(wd >> 5) & 31} nocollide {(wd >> 10) & 31} group {wd >> 16:#x}"
def up(q):
    x, y, z, w = q
    return (2 * (x * y - w * z), 1 - 2 * (x * x + z * z), 2 * (y * z + w * x))
with open(sys.argv[1], "w") as fh:
    w = lambda s: (fh.write(s + "\n"), fh.flush())
    seen = {}; nxt = 0
    while time.time() - t0 < secs:
        now = time.time()
        if now < nxt: time.sleep(0.01); continue
        nxt = now + 0.25; hp = H.update(); row = []
        for gd in guids:
            a = H.state_addr(gd); b = g.read(a, 0x20) if a else None
            if not b or len(b) < 0x20: continue
            x, y, z, c, qx, qy, qz, qw = struct.unpack(">3fI4f", b)
            if gd not in seen:
                rec, P, how = pc.lookup(g, H, gd); bl = pc.bodies(g, P)[3] if P else []
                seen[gd] = bl
                fw = [dec(pc.u32(g, pc.u32(g, gb + 0x34) + 0x2C)) for gb, B in bl if pc.ok_ptr(pc.u32(g, gb + 0x34))]
                w(f"FIRST {gd:#x} t {now-t0:.2f} pos ({x:.4f},{y:.4f},{z:.4f}) quat ({qx:.5f},{qy:.5f},{qz:.5f},{qw:.5f}) P {P and hex(P)} vt {pc.u32(g, P) if P else 0:#x} bodies {len(bl)} filters {fw}")
            bl = seen[gd]; v = pc.body_state(g, bl[0][1])[0] if bl and pc.ok_ptr(bl[0][1]) else None
            yaw = math.degrees(math.atan2(2 * (qw * qy + qx * qz), 1 - 2 * (qy * qy + qx * qx))); u = up((qx, qy, qz, qw))
            row.append(f"{gd:#x} pos ({x:.4f},{y:.4f},{z:.4f}) yaw {yaw:.3f} up ({u[0]:.4f},{u[1]:.4f},{u[2]:.4f}) v {v and tuple(round(q, 3) for q in v)}")
        if row: w(f"t {now-t0:.2f} her {hp and tuple(round(q, 1) for q in hp)} | " + " | ".join(row))
