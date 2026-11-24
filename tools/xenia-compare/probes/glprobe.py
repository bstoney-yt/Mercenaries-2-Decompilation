# glprobe.py <outdir> <seconds>: read-only, GL #715. Every 0.5 s and when a route mark appears in route.log: snapshot her store-record
# P (0x400 bytes) and every heap object its first 0x400 bytes point to (0x200 each) to snap_<n>.bin + index; offline, diff the
# snapshots around one RT (glsnap1 -> glsnap2) and LT+RT (glsnap2 -> glsnap3) for a small counter that drops by one (6 -> 5).
import sys, struct, subprocess, time, os
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
sys.path.insert(0, "C:/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad")
import xmem, physchain as pc
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g); od, secs = sys.argv[1], float(sys.argv[2]); os.makedirs(od, exist_ok=True); t0 = time.time()
log = os.path.join(os.path.dirname(od.rstrip("/")), "route.log") if False else os.path.join(od, "route.log")
idx = open(os.path.join(od, "glsnaps.txt"), "w"); seen_marks = set(); n = 0
while time.time() - t0 < secs:
    marks = set()
    try:
        for l in open(log, encoding="latin-1"):
            if " mark glsnap" in l: marks.add(l.split("mark ")[1].strip())
    except OSError: pass
    new = marks - seen_marks
    if new:
        seen_marks |= new
        hero = H.character(); rec, P, how = pc.lookup(g, H, hero)
        blob = g.read(P, 0x400) if P else b""; parts = [("P", P, blob)]
        for o in range(0, len(blob) - 3, 4):
            q = struct.unpack_from(">I", blob, o)[0]
            if pc.ok_ptr(q): parts.append((f"P+{o:#x}", q, g.read(q, 0x200) or b""))
        for tag, a, b in parts:
            idx.write(f"{sorted(new)} {tag} {a:#x} {b.hex()}\n")
        idx.flush(); n += 1
    time.sleep(0.1)
