# copylog.py <out.txt>: her position copies before and after one short proven poke (ChiCon001, phantom+0xD0 -> (-2618,-15,405)).
# Read-only except that single poke (dry-run gated). Finds every 4-aligned address holding her exact x (BE float) with y/z within 2 m
# (plus the camera block *(0x83457050) position via xmem.Camera), logs all of them every 0.1 s for 10 s, pokes, logs 20 s more.
import sys, struct, subprocess, time, math
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
sys.path.insert(0, "C:/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad")
import xmem, physchain as pc
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g); u32 = lambda a: pc.u32(g, a); cam = xmem.Camera(g)
fh = open(sys.argv[1], "w"); w = lambda s: (fh.write(s + "\n"), fh.flush())
ph = 0; tw = time.time()
while time.time() - tw < 60:
    hero = H.character(); rec, P, how = pc.lookup(g, H, hero) if hero else (None, None, None)
    ph = u32(u32(P + 0x4C) + 0x34) if P else 0
    if pc.ok_ptr(ph) and u32(ph) == 0x82051FF4 and H.update(): break
    time.sleep(0.5)
time.sleep(3)  # let her stand still for the scan
hp = H.update(); xw = struct.unpack(">I", struct.pack(">f", hp[0]))[0]
hits = xmem.scan_word(g, xw, 0x40000000, 0xC0000000, 128) + xmem.scan_word(g, xw, 0x82000000, 0xA0000000, 64)
copies = []
for a in hits:
    t = pc.f3(g, a)
    if t and abs(t[2] - hp[2]) < 2 and abs(t[1] - hp[1]) < 200: copies.append((a, round(t[1] - hp[1], 3), u32(a - 4)))
w(f"her {hp} phantom {ph:#x} copies {len(copies)}: " + " ".join(f"{a:#x}(dy {dy:+}, word-4 {wb:#x})" for a, dy, wb in copies))
def snap(tag):
    row = f"{tag} t {time.time()-tw:.2f} rec {tuple(round(v, 2) for v in H.update())} cam {cam.pos() and tuple(round(v, 2) for v in cam.pos())}"
    for a, dy, wb in copies:
        t = pc.f3(g, a); row += f" | {a:#x} {tuple(round(v, 2) for v in t) if t else None}"
    w(row)
t0 = time.time()
while time.time() - t0 < 10: snap("pre"); time.sleep(0.1)
cur = pc.f3(g, ph + 0xD0); recp = H.update()
if not cur or math.dist(cur, recp) > 0.05: w("ABORT: dry run mismatch"); sys.exit(1)
w(f"DRY RUN OK; WRITE phantom+0xD0 = (-2618, -15, 405) ok {g.write(ph + 0xD0, struct.pack('>3f', -2618.0, -15.0, 405.0))}")
t0 = time.time()
while time.time() - t0 < 20: snap("post"); time.sleep(0.1)
