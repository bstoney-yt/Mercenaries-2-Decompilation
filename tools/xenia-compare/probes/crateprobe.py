# crateprobe.py <out.csv> <seconds> <x> <z> <r>: read-only; from mission load, every ~0.1 s, every state-table entry within r m
# (flat) of (x, z): logged when first seen and whenever it moves more than 2 cm (index, position, quaternion, counter)
import sys, struct, subprocess, time, math, csv
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
import xmem
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g)
secs, X, Z, R = float(sys.argv[2]), float(sys.argv[3]), float(sys.argv[4]), float(sys.argv[5])
t0 = time.time(); last = {}
with open(sys.argv[1], "w", newline="") as fh:
    wr = csv.writer(fh); wr.writerow(["t", "index", "x", "y", "z", "counter", "qx", "qy", "qz", "qw", "her_x", "her_z"])
    while time.time() - t0 < secs:
        her = H.update() or (0, 0, 0)
        hdr = g.read(H.STATES, 0x80)
        if hdr:
            mask = struct.unpack_from(">I", hdr, 0x20)[0]; esz, shift = struct.unpack_from(">hh", hdr, 0x24)
            pages = struct.unpack_from(">I", hdr, 0x70)[0]; cap = struct.unpack_from(">I", hdr, 0x1C)[0]
            n_pages = (cap + mask - 1) // mask if mask else 0
            pp = g.read(pages, 4 * n_pages) if n_pages else None
            for pi, page in enumerate(struct.unpack(f">{n_pages}I", pp) if pp else ()):
                if not page: continue
                b = g.read(page, mask * esz)
                if not b: continue
                for j in range(mask):
                    x, y, z, c, qx, qy, qz, qw = struct.unpack_from(">3fI4f", b, j * esz)
                    if math.hypot(x - X, z - Z) <= R and abs(y) < 2000:
                        idx = (pi << shift) | j
                        lp = last.get(idx)
                        if lp is None or math.dist(lp, (x, y, z)) > 0.02:
                            last[idx] = (x, y, z)
                            wr.writerow([f"{time.time()-t0:.2f}", idx, f"{x:.3f}", f"{y:.3f}", f"{z:.3f}", c, f"{qx:.4f}", f"{qy:.4f}", f"{qz:.4f}", f"{qw:.4f}", f"{her[0]:.1f}", f"{her[2]:.1f}"])
        fh.flush(); time.sleep(0.1)
