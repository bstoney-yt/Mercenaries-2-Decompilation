# nearrpg.py <out.txt> <seconds>: read-only. Every 1 s: her position, and for the RPG trio 0x10E2FC/FD/FE whether each has a state record
# (loaded) and its position and distance from her.
import sys, struct, subprocess, time, math
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
import xmem
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g); secs = float(sys.argv[2]); t0 = time.time(); IDS = (0x10E2FC, 0x10E2FD, 0x10E2FE)
with open(sys.argv[1], "w") as fh:
    while time.time() - t0 < secs:
        hp = H.update(); row = f"t {time.time()-t0:.1f} her {hp and tuple(round(v, 1) for v in hp)}"
        for gd in IDS:
            a = H.state_addr(gd); p = struct.unpack(">3f", g.read(a, 12)) if a else None
            row += f" | {gd:#x} " + (f"loaded ({p[0]:.1f},{p[1]:.1f},{p[2]:.1f}) d {math.dist(p, hp):.0f}" if p and hp else "not loaded")
        fh.write(row + "\n"); fh.flush(); time.sleep(1)
