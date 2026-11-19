# hpfast.py <out.csv> <car x> <car z> <seconds>: every change of her health and the car's (polled ~1 ms)
import sys, struct, subprocess, math, time, csv
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
import xmem
pid = None
while pid is None:
    out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
    pid = next((int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower()), None)
    time.sleep(2)
g = xmem.Guest(pid); H = xmem.Hero(g)
x, z = float(sys.argv[2]), float(sys.argv[3])
car = hero = None
while car is None or hero is None:
    try:
        hero = H.character()
        hm = xmem.map_entries(g, 0x83656558)
        for k, a in hm.items():
            if k >> 28 != 4: continue
            sa = H.state_addr(k)
            if sa:
                p = struct.unpack(">3f", g.read(sa, 12))
                if math.hypot(p[0]-x, p[2]-z) < 1.0 and struct.unpack(">f", g.read(a, 4))[0] == 100.0:
                    car = (k, a)
    except Exception: pass
    time.sleep(1)
ha = xmem.map_entries(g, 0x83656558)[hero]
print("hero", hex(hero), "car", hex(car[0]), flush=True)
end = time.time() + float(sys.argv[4]); t0 = time.time(); last = None
with open(sys.argv[1], "w", newline="") as fh:
    wr = csv.writer(fh); wr.writerow(["t", "hero_hp", "car_hp", "hx", "hy", "hz", "dt", "frame"])
    while time.time() < end:
        hv = struct.unpack(">f", g.read(ha + 4, 4))[0]; cv = struct.unpack(">f", g.read(car[1] + 4, 4))[0]
        if (hv, cv) != last:
            last = (hv, cv); p = H.update() or (0, 0, 0)
            dtb = g.read(0x82B7C730, 4); sa = H.state_addr(hero); fc = g.read(sa + 12, 4) if sa else None
            wr.writerow([f"{time.time()-t0:.4f}", f"{hv:.3f}", f"{cv:.3f}", *(f"{c:.2f}" for c in p),
                         f"{struct.unpack('>f', dtb)[0]:.5f}" if dtb else "", struct.unpack('>I', fc)[0] if fc else ""]); fh.flush()
        time.sleep(0.0005)
