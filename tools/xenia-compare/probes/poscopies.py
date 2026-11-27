import sys, struct, subprocess, time
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
sys.path.insert(0, "C:/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad")
import xmem, physchain as pc
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g); u32 = lambda a: pc.u32(g, a)
hero = H.character(); sa = H.state_addr(hero); xb = g.read(sa, 4); hp = struct.unpack(">3f", g.read(sa, 12))
word = struct.unpack(">I", xb)[0]
hits = xmem.scan_word(g, word, 0x40000000, 0xC0000000, 64)
rec, P, how = pc.lookup(g, H, hero); ph = u32(u32(P + 0x4C) + 0x34)
print("her", hp, "state rec", hex(sa), "phantom", hex(ph), "x word", hex(word), "copies", len(hits))
for a in hits:
    t = struct.unpack(">3f", g.read(a, 12)); 
    near = [f"{n}+{a-b:#x}" for n, b in (("state", sa), ("phantom", ph), ("P", P)) if 0 <= a - b < 0x1000]
    print(f"  {a:#x} triple ({t[0]:.4f},{t[1]:.4f},{t[2]:.4f}) y-match {abs(t[1]-hp[1])<0.01} z-match {abs(t[2]-hp[2])<0.01} {near} word-before {u32(a-4):#x}")
