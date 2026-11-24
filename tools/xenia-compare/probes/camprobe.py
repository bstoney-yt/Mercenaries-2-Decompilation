# camprobe.py <out.txt> <seconds>: read-only. Every 0.25 s (and on every change of camera object): the active camera
# *(u32*)0x83457050, its vtable and raw 0x100 bytes (its params; camlog gives yaw/pitch/dt at 50 Hz alongside).
import sys, struct, subprocess, time
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
import xmem
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); secs = float(sys.argv[2]); t0 = time.time(); u = lambda a: struct.unpack(">I", g.read(a, 4) or bytes(4))[0]
with open(sys.argv[1], "w") as fh:
    last = None; nxt = 0
    while time.time() - t0 < secs:
        cam = u(0x83457050); now = time.time()
        if cam != last or now >= nxt:
            raw = g.read(cam, 0x100) if cam else None
            fh.write(f"t {now-t0:.3f} cam {cam:#x} vt {u(cam) if cam else 0:#x} raw {raw.hex() if raw else None}\n"); fh.flush()
            last, nxt = cam, now + 0.25
        time.sleep(0.01)
