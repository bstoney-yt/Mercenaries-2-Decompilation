# stepdt.py <out.txt> <seconds>: read-only. Per change of the hero's state counter: the frame dt global 0x82B7C730, the physics
# manager (u32(0x831334C8)) dt at +0x2390 and its step counter +0x23D4 (0x82153238 steps once with dt = +0x2390, then +0x23D4 += 1).
import sys, struct, subprocess, time
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
import xmem
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g); secs = float(sys.argv[2]); t0 = time.time()
f = lambda a: struct.unpack(">f", g.read(a, 4) or bytes(4))[0]; u = lambda a: struct.unpack(">I", g.read(a, 4) or bytes(4))[0]
with open(sys.argv[1], "w") as fh:
    last = None; mgr = u(0x831334C8); fh.write(f"manager {mgr:#x}\n")
    while time.time() - t0 < secs:
        h = H.character(); a = H.state_addr(h) if h else None
        c = u(a + 0xC) if a else None
        if c == last: continue
        last = c
        fh.write(f"t {time.time()-t0:.4f} hero counter {c} dt {f(0x82B7C730):.5f} phys dt {f(mgr + 0x2390):.5f} steps {u(mgr + 0x23D4)}\n")
