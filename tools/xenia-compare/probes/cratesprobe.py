# cratesprobe.py <outdir> <seconds>: read-only, #680 crates (OilCon020, truck 0xFA056). Start at mission load.
# crates.csv: per truck counter, the truck's position/quaternion and body v/w, and per crate: position, quaternion, motion byte
#   B+0xD8, v B+0x1A0, w B+0x1B0 (chain: store 0x836571D4[guid]+0 = P; P+0x4C/+0x50 bodies; wrapper+0x38 = B).
# setup.txt: each crate's first-seen position and its position 1.5 s later; P/B/wrapper; the render link (table 0x83656008
#   kind 0xC26AD8AB, raw); every gravity action (vtable 0x820C8284) found, its scale +0x28, and which crate body/wrapper it points at.
import sys, struct, subprocess, time, math, csv, os
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
sys.path.insert(0, "C:/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad")
import xmem, physchain as pc
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g)
od, secs = sys.argv[1], float(sys.argv[2]); os.makedirs(od, exist_ok=True)
TRUCK = 0xFA056
CRATES = list(range(0x11D397, 0x11D3A0)) + list(range(0x1225F2, 0x1225FC))
GRAV_VT = 0x820C8284
t0 = time.time()
st = open(os.path.join(od, "setup.txt"), "w")
say = lambda s: (st.write(f"{time.time()-t0:8.3f} {s}\n"), st.flush())
def rec(guid):
    a = H.state_addr(guid)
    b = g.read(a, 0x20) if a else None
    return struct.unpack(">3fI4f", b) if b and len(b) == 0x20 else None
def phys(guid):
    r, P, how = pc.lookup(g, H, guid)
    if not P: return None
    lst, n, ind, bl = pc.bodies(g, P)
    return P, how, [(w, B) for w, B in bl if pc.ok_ptr(B)]
def bstate(B):
    b = g.read(B + 0xD8, 1); v, w = pc.body_state(g, B)
    return (b[0] if b else -1), v, w
# the truck and the parked vehicles near it: first sight, +1.5 s, +10 s (layer origin y -33.77; crates 0x11D397 -32.95, 0x1225FA -32.83)
def vehicles_near(tr, radius=60.0):
    idx = H.table_index(H.STATES); store = xmem.map_entries(g, pc.STORE); out = {}
    for h, a in idx.items():
        b = g.read(a, 0x20)
        if not b or len(b) < 0x20: continue
        x, y, z = struct.unpack_from(">3f", b, 0)
        if math.hypot(x - tr[0], z - tr[2]) > radius or h >= 0x40000000: continue  # layer objects only
        r = store.get(h); P = pc.u32(g, r) if r else 0
        if P and pc.u32(g, P) == pc.CAR_VT: out[h] = True
    return sorted(out)
def log_pose(tag, guids):
    for h in guids:
        r = rec(h)
        say(f"{tag} {h:#x} pos {r and '(' + ','.join(f'{v:.4f}' for v in r[:3]) + ')'} quat {r and tuple(round(q, 6) for q in r[4:])} counter {r and r[3]}")
while time.time() - t0 < secs and not rec(TRUCK): time.sleep(0.01)
tr0 = rec(TRUCK); vguids = [TRUCK] + [h for h in vehicles_near(tr0) if h != TRUCK][:4] if tr0 else [TRUCK]
say(f"vehicles logged (layer guids with car-class physics within 60 m of the truck): {[hex(h) for h in vguids]}")
log_pose("first-sight", vguids); t_seen = time.time()
# wait for the truck and the crates to exist
first = {}
while time.time() - t0 < secs and len(first) < len(CRATES):
    for c in CRATES:
        if c not in first:
            r = rec(c)
            if r: first[c] = (time.time() - t0, r); say(f"crate {c:#x} first seen pos ({r[0]:.3f},{r[1]:.3f},{r[2]:.3f}) quat {tuple(round(q,4) for q in r[4:])}")
    if first and time.time() - t0 - min(v[0] for v in first.values()) > 20: break  # 20 s after the first crate appears
    time.sleep(0.02)
tr = rec(TRUCK); say(f"truck {TRUCK:#x} pos {tr and tuple(round(v,3) for v in tr[:3])} quat {tr and tuple(round(q,4) for q in tr[4:])}; crates seen {len(first)}/{len(CRATES)}")
time.sleep(1.5)
log_pose("vehicle(after crates seen +1.5 s)", vguids)
att = xmem.map_entries(g, 0x83656008)
info = {}
for c in sorted(first):
    r = rec(c); p = phys(c); a = att.get(c); ab = g.read(a, 0x30) if a else None
    d = [r[i] - first[c][1][i] for i in range(3)] if r else None
    info[c] = p
    say(f"crate {c:#x} +1.5s pos {r and tuple(round(v,3) for v in r[:3])} moved {d and tuple(round(v,3) for v in d)}; P {p and hex(p[0])} via {p and p[1]} "
        f"vt {p and hex(pc.u32(g, p[0]))} bodies {p and [(hex(w), hex(B), bstate(B)[0]) for w, B in p[2]]}; link {a and hex(a)} raw {ab.hex() if ab else None}")
tp = phys(TRUCK); say(f"truck P {tp and hex(tp[0])} vt {tp and hex(pc.u32(g, tp[0]))} bodies {tp and [(hex(w), hex(B)) for w, B in tp[2]]}")
# gravity actions: scan for the vtable, then match the action's words against every crate body / wrapper / P
def scan_actions():
    acts = xmem.scan_word(g, GRAV_VT, 0x40000000, 0xC0000000, 256)
    owners = {}
    for c, p in info.items():
        if not p: continue
        owners[p[0]] = (c, "P")
        for w, B in p[2]: owners[w] = (c, "wrapper"); owners[B] = (c, "body")
    for a in acts:
        b = g.read(a, 0x60) or b""
        words = struct.unpack(f">{len(b)//4}I", b[: len(b)//4*4])
        hit = [(f"+{4*i:#x}", hex(wd), *owners[wd]) for i, wd in enumerate(words) if wd in owners]
        sc = struct.unpack_from(">f", b, 0x28)[0] if len(b) >= 0x2C else float("nan")
        say(f"gravity action {a:#x} scale+0x28 {sc:.3f} refs {hit}")
    say(f"gravity actions found: {len(acts)}")
scan_actions()
while time.time() - t_seen < 10: time.sleep(0.05)
log_pose("vehicle(+10 s)", vguids)
# per-frame log
with open(os.path.join(od, "crates.csv"), "w", newline="") as fh:
    wr = csv.writer(fh)
    wr.writerow(["t", "truck_c", "tx", "ty", "tz", "tqx", "tqy", "tqz", "tqw", "tvx", "tvy", "tvz", "twx", "twy", "twz", "crate", "x", "y", "z", "qx", "qy", "qz", "qw", "motion", "vx", "vy", "vz", "wx", "wy", "wz"])
    last = None; rescan = time.time() + 20
    while time.time() - t0 < secs:
        tr = rec(TRUCK)
        if not tr or tr[3] == last: continue
        last = tr[3]
        tv = tw = (float("nan"),) * 3
        if tp and tp[2]:
            v, w = pc.body_state(g, tp[2][0][1]); tv, tw = v or tv, w or tw
        now = f"{time.time()-t0:.4f}"
        for c in CRATES:
            r = rec(c)
            if not r: continue
            p = info.get(c)
            mb, v, w = bstate(p[2][0][1]) if p and p[2] else (-1, None, None)
            wr.writerow([now, tr[3], *(round(x, 4) for x in tr[:3]), *(round(q, 5) for q in tr[4:]), *(round(x, 3) for x in tv), *(round(x, 3) for x in tw),
                         hex(c), *(round(x, 4) for x in r[:3]), *(round(q, 5) for q in r[4:]), mb, *(round(x, 3) for x in (v or (float('nan'),)*3)), *(round(x, 3) for x in (w or (float('nan'),)*3))])
        fh.flush()
        if time.time() > rescan:  # physics objects may be rebuilt when she gets in
            for c in info:
                if not info[c]: info[c] = phys(c)
            if not tp: tp = phys(TRUCK)
            rescan = time.time() + 20
