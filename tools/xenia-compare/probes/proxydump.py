# proxydump.py: read-only. Her store record P -> list -> game body gb (u32(list) or inline) -> hk = u32(gb+0x34); dump P, gb, hk and
# the objects hk points to; mark every float triple within 0.5 m of her state-record position (and its offset).
import sys, struct, subprocess, math
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
sys.path.insert(0, "C:/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad")
import xmem, physchain as pc
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g); u32 = lambda a: pc.u32(g, a)
hero = H.character(); hp = H.update(); rec, P, how = pc.lookup(g, H, hero)
print("hero", hex(hero), "pos", hp, "P", hex(P), "vt", hex(u32(P)), "list", hex(u32(P + 0x4C)), "n", u32(P + 0x50), "arr", hex(u32(P + 0x54)))
def scan(tag, a, n=0x300):
    b = g.read(a, n) or b""
    hits = []
    for o in range(0, len(b) - 11, 4):
        t = struct.unpack_from(">3f", b, o)
        if all(math.isfinite(x) for x in t) and math.dist(t, hp) < 0.5: hits.append(f"+{o:#x}=({t[0]:.3f},{t[1]:.3f},{t[2]:.3f})")
    print(f"{tag} {a:#x} vt {u32(a):#x} pos-matches: {hits}")
    return b
lst = u32(P + 0x4C)
for gtag, gb in (("gb=u32(list)", u32(lst)), ("gb=list", lst)):
    if not pc.ok_ptr(gb): continue
    scan(gtag, gb, 0x100)
    hk = u32(gb + 0x34)
    print(f"  {gtag}+0x34 hk {hk:#x} vt {u32(hk) if pc.ok_ptr(hk) else 0:#x} +0x2C filter {u32(hk + 0x2C) if pc.ok_ptr(hk) else 0:#x} gb+0x38 {u32(gb + 0x38):#x}")
    if pc.ok_ptr(hk):
        b = scan("    hk", hk, 0x300)
        for o in range(0, 0x100, 4):
            q = struct.unpack_from(">I", b, o)[0]
            if pc.ok_ptr(q) and q >= 0x40000000: scan(f"      hk+{o:#x} ->", q, 0x200)
scan("P", P, 0x300)
