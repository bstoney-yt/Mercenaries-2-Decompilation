# carhold.py (boatprobe at 0.25 s) <out.txt> <seconds> <guid hex>...: read-only, #766 moored-boat drift (Jennifer). Every 0.5 s (and on first sight):
# each boat's state record position, yaw (deg, from the quaternion) and up vector (quaternion applied to +y), plus her position.
import sys, struct, subprocess, time, math
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
import xmem
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g); secs = float(sys.argv[2]); guids = [int(x, 16) for x in sys.argv[3:]]; t0 = time.time()
def up(q):
    x, y, z, w = q
    return (2 * (x * y - w * z), 1 - 2 * (x * x + z * z), 2 * (y * z + w * x))
with open(sys.argv[1], "w") as fh:
    seen = set(); nxt = 0
    while time.time() - t0 < secs:
        now = time.time(); hp = H.update()
        rows = []
        for gd in guids:
            a = H.state_addr(gd); b = g.read(a, 0x20) if a else None
            if not b or len(b) < 0x20: continue
            x, y, z, c, qx, qy, qz, qw = struct.unpack(">3fI4f", b)
            yaw = math.degrees(math.atan2(2 * (qw * qy + qx * qz), 1 - 2 * (qy * qy + qx * qx))); u = up((qx, qy, qz, qw))
            first = gd not in seen; seen.add(gd)
            rows.append(f"{gd:#x}{' FIRST' if first else ''} c {c} pos ({x:.4f},{y:.4f},{z:.4f}) yaw {yaw:.3f} up ({u[0]:.4f},{u[1]:.4f},{u[2]:.4f})"
                        + (f" dist {math.hypot(x - hp[0], z - hp[2]):.1f}" if hp else ""))
        if rows and (now >= nxt or any("FIRST" in r for r in rows)):
            fh.write(f"t {now - t0:.2f} her {hp and tuple(round(v, 1) for v in hp)} | " + " | ".join(rows) + "\n"); fh.flush(); nxt = now + 0.25
        time.sleep(0.02)
