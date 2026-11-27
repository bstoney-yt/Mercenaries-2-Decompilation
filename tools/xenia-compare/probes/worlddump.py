# worlddump.py: read-only, once. The Havok world object: u32(her proxy phantom + 8) (store P -> inline wrapper P+0x4C -> +0x34 -> +8),
# cross-checked with u32(B+8) of any car body seen by the run's own probes. Writes vtable + 0x200 bytes (hex and BE words) to
# Z:\mercs2\scratch\vehicles\havok-world-dump.txt (LAN only, never committed). Skips if that file exists.
import sys, struct, subprocess, os, time
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
sys.path.insert(0, "C:/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad")
import xmem, physchain as pc
OUT = "Z:/mercs2/scratch/vehicles/havok-world-dump2.txt"
if os.path.exists(OUT): sys.exit()
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g); u32 = lambda a: pc.u32(g, a)
for _ in range(60):
    hero = H.character(); rec, P, how = pc.lookup(g, H, hero) if hero else (None, None, None)
    ph = u32(u32(P + 0x4C) + 0x34) if P else 0
    world = u32(ph + 8) if pc.ok_ptr(ph) else 0
    if pc.ok_ptr(world): break
    time.sleep(1)
b = g.read(world, 0x500) if pc.ok_ptr(world) else None
with open(OUT, "w") as fh:
    fh.write(f"# Havok world object, read-only dump {time.strftime('%Y-%m-%d %H:%M:%S')} (Jennifer run). Via her proxy phantom {ph:#x} +8.\n")
    fh.write(f"world {world:#x} vtable {u32(world):#x}\n")
    if b:
        fh.write("raw " + b.hex() + "\n")
        for o in range(0, len(b), 16):
            fh.write(f"+{o:#05x}: " + " ".join(f"{w:08x}" for w in struct.unpack_from('>4I', b, o)) + "\n")
# collision input: the hkpWorld member order fits gravity +0x10, pendingOperations +0x7C, pendingOperationsCount +0x80, critical-ops
# lock +0x84, so +0x6C = m_collisionInput (chosen); +0x68 (the member before it) is dumped as a fallback.
if b:
    with open(OUT, "a") as fh:
        for off in (0x7C, 0x6C, 0x68):  # 0x7C = collision input per the world ctor 0x82624898 / 0x826425C8 (vehicle thread)
            q = struct.unpack_from(">I", b, off)[0]; cb = g.read(q, 0x100) if pc.ok_ptr(q) else None
            fh.write(f"world+{off:#x} -> {q:#x} vtable {u32(q) if pc.ok_ptr(q) else 0:#x}\n")
            if cb:
                fh.write("raw " + cb.hex() + "\n")
                for o in range(0, len(cb), 16):
                    fh.write(f"  +{o:#05x}: " + " ".join(f"{x:08x}" for x in struct.unpack_from(">4I", cb, o)) + "  | " + " ".join(f"{x:.6g}" for x in struct.unpack_from(">4f", cb, o)) + "\n")
