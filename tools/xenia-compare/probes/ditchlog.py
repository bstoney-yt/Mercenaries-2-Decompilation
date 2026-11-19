# ditchlog.py <out.csv> <seconds>: per game frame, her record and her car's record (position + quaternion), frame dt, her state
import sys, struct, subprocess, time, csv, math
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
import xmem
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g); h = H.character()
car = None
while car is None:  # start before she gets in: wait until she is seated (stance 0x5E2CD838), then take the record under her
    st = H.anim_state(h)
    if st and st[0] == 0x5E2CD838:
        here = H.update()
        for k in xmem.map_entries(g, 0x83656558):
            if h - 1 <= k <= h + 8: continue
            sa = H.state_addr(k)
            if sa and math.dist(struct.unpack(">3f", g.read(sa, 12)), here) < 1.0: car = k; break
    time.sleep(0.05)
print("hero", hex(h), "car", car and hex(car), flush=True)
end = time.time() + float(sys.argv[2]); t0 = time.time(); last = None
with open(sys.argv[1], "w", newline="") as fh:
    wr = csv.writer(fh); wr.writerow(["t", "frame", "dt", "hx", "hy", "hz", "cx", "cy", "cz", "cqx", "cqy", "cqz", "cqw", "stance", "action", "mode"])
    while time.time() < end:
        sa = H.state_addr(h); b = g.read(sa, 32) if sa else None
        if b and len(b) == 32:
            hf = struct.unpack(">3fI4f", b)
            if hf[3] != last:
                last = hf[3]
                ca = H.state_addr(car) if car else None; cb = g.read(ca, 32) if ca else None
                cf = struct.unpack(">3fI4f", cb) if cb and len(cb) == 32 else (float("nan"),) * 8
                dtb = g.read(0x82B7C730, 4); st = H.anim_state(h) or (0, 0, 0)
                wr.writerow([f"{time.time()-t0:.4f}", hf[3], f"{struct.unpack('>f', dtb)[0]:.5f}" if dtb else "",
                             *(f"{v:.4f}" for v in hf[0:3]), *(f"{v:.4f}" for v in cf[0:3]), *(f"{v:.5f}" for v in cf[4:8]),
                             *(f"{v:#010x}" for v in st)])
        time.sleep(0.0005)
