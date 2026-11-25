import sys, struct, subprocess
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
sys.path.insert(0, "C:/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad")
import xmem, physchain as pc
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g); u32 = lambda a: pc.u32(g, a)
hero = H.character(); hp = H.update(); rec, P, how = pc.lookup(g, H, hero); hk = u32(u32(P + 0x4C) + 0x34)
print("her record", hp, "hk", hex(hk), "vt", hex(u32(hk)))
b = g.read(hk + 0x80, 0x140)
for o in range(0, 0x140, 0x10):
    f = struct.unpack_from(">4f", b, o); print(f"  hk+{0x80+o:#05x}: " + " ".join(f"{x:11.4f}" for x in f))
