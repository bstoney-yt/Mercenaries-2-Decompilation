# contactlog.py <out.csv> <seconds> <radius>: every game frame, her health and record, and every state-table record
# within <radius> m of her (a passing vehicle and its crew), to fit per-frame contact damage against vehicle speed
import sys, struct, subprocess, time, csv, math
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
import xmem
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g); h = H.character(); ha = xmem.map_entries(g, 0x83656558)[h]
R = float(sys.argv[3]); end = time.time() + float(sys.argv[2]); t0 = time.time(); last = None
with open(sys.argv[1], "w", newline="") as fh:
    wr = csv.writer(fh); wr.writerow(["t", "frame", "dt", "hp", "hx", "hy", "hz", "near"])
    while time.time() < end:
        sa = H.state_addr(h); b = g.read(sa, 16) if sa else None
        if not b or len(b) < 16: time.sleep(0.001); continue
        hx, hy, hz, fc = struct.unpack(">3fI", b)
        if fc == last: time.sleep(0.0005); continue
        last = fc
        hv = struct.unpack(">f", g.read(ha + 4, 4))[0]; dtb = g.read(0x82B7C730, 4)
        near = [(i, p) for i, p in xmem.state_entries(g).items() if 0.3 < math.dist(p, (hx, hy, hz)) < R]
        # each nearby record with its quaternion (yaw rate offline from successive frames)
        cells = []
        for i, p in near:
            q = xmem.state_quat(g, i)
            cells.append(f"{i}:{p[0]:.3f},{p[1]:.3f},{p[2]:.3f}" + (f",{q[0]:.4f},{q[1]:.4f},{q[2]:.4f},{q[3]:.4f}" if q else ""))
        wr.writerow([f"{time.time()-t0:.4f}", fc, f"{struct.unpack('>f', dtb)[0]:.5f}" if dtb else "", f"{hv:.3f}",
                     f"{hx:.3f}", f"{hy:.3f}", f"{hz:.3f}", ";".join(cells)])
        fh.flush()
