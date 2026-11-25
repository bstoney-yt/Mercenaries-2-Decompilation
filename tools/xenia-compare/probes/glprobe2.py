# glprobe2.py <outdir> <seconds>: read-only, GL #715 retry (Jennifer). At each route mark "glsnap*" (route.log in <outdir>): snapshot
# (1) the player object (players pool 0x83657FA8 slot whose +0x14 is her entity) 0x400 + its pointees 0x200 each,
# (2) her state-machine object S = u32(entry of table 0x83657B94) 0x400 + pointees, (3) her entity's store record P 0x400 + pointees.
# Offline: diff glsnap1 -> 2 (one RT) and 2 -> 3 (LT+RT) for a counter dropping by one. Ordnance proof: run ordlog.py alongside.
import sys, struct, subprocess, time, os
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
sys.path.insert(0, "C:/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad")
import xmem, physchain as pc
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g); od, secs = sys.argv[1], float(sys.argv[2]); os.makedirs(od, exist_ok=True); t0 = time.time()
u32 = lambda a: pc.u32(g, a)
def player_obj():
    pool = H.PLAYERS
    for i in range(min(u32(pool + 0x18), 64)):
        o = u32(H._slot(pool, i))
        if o and u32(o + 0x20) == 0: return o
    return 0
def tree(tag, a):
    blob = g.read(a, 0x400) if pc.ok_ptr(a) else b""; parts = [(tag, a, blob or b"")]
    for o in range(0, len(blob or b"") - 3, 4):
        q = struct.unpack_from(">I", blob, o)[0]
        if pc.ok_ptr(q): parts.append((f"{tag}+{o:#x}", q, g.read(q, 0x200) or b""))
    return parts
idx = open(os.path.join(od, "glsnaps2.txt"), "w"); seen = set()
while time.time() - t0 < secs:
    marks = set()
    try:
        for l in open(os.path.join(od, "route.log"), encoding="latin-1"):
            if " mark glsnap" in l: marks.add(l.split("mark ")[1].strip())
    except OSError: pass
    for m in sorted(marks - seen):
        seen.add(m); hero = H.character()
        sa = H.state_addr(hero, 0x83657B94); Sm = u32(sa) if sa else 0
        rec, P, _ = pc.lookup(g, H, hero)
        for tag, a, b in tree("player", player_obj()) + tree("statemachine", Sm) + tree("storeP", P or 0):
            if b: idx.write(f"{m} {tag} {a:#x} {b.hex()}\n")
        idx.flush()
    time.sleep(0.1)
