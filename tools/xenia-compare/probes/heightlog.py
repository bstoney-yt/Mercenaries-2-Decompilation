# heightlog.py <out.csv> <seconds>: her state record (x,y,z, frame counter) + frame dt + stance/action/mode, on every change (~1 ms poll)
import sys, struct, subprocess, time, csv
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
import xmem
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g); h = H.character()
end = time.time() + float(sys.argv[2]); t0 = time.time(); last = None
with open(sys.argv[1], "w", newline="") as fh:
    wr = csv.writer(fh); wr.writerow(["t", "frame", "dt", "x", "y", "z", "stance", "action", "mode"])
    while time.time() < end:
        sa = H.state_addr(h); b = g.read(sa, 16) if sa else None
        if b and len(b) == 16:
            x, y, z, fc = struct.unpack(">3fI", b)
            if fc != last:
                last = fc
                dtb = g.read(0x82B7C730, 4); st = H.anim_state(h) or (0, 0, 0)
                wr.writerow([f"{time.time()-t0:.4f}", fc, f"{struct.unpack('>f', dtb)[0]:.5f}" if dtb else "", f"{x:.3f}", f"{y:.4f}", f"{z:.3f}", *(f"{v:#010x}" for v in st)])
        time.sleep(0.0005)
