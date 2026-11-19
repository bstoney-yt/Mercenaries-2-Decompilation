# hiber.py <out.txt> <seconds> <car x> <car z> <guid hex>...: read-only, every 0.5 s: for the car (found by its parked spot)
# and each given guid: present in the object table 0x8365808C? its transform record position; its state-table record
# (0x83658140) position; its health (pool 0x83656558); plus the removal queue 0x8357F9B8's first 0x40 bytes and the grid
# level bytes (0x83134224 + cell x 0x1F0, +0x80) for cells 0..63 (logged when any changes), and her position.
import sys, struct, subprocess, time, math
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
import xmem
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g)
secs, cx, cz = float(sys.argv[2]), float(sys.argv[3]), float(sys.argv[4])
guids = [int(x, 16) for x in sys.argv[5:]]
hm = xmem.map_entries(g, 0x83656558)
for k, a in hm.items():
    sa = H.state_addr(k)
    if sa:
        p = struct.unpack(">3f", g.read(sa, 12))
        if math.hypot(p[0] - cx, p[2] - cz) < 1.0 and struct.unpack(">f", g.read(a, 4))[0] == 100.0:
            guids.insert(0, k); break
t0 = time.time(); last = {}
with open(sys.argv[1], "w") as fh:
    w = lambda s: (fh.write(s + "\n"), fh.flush())
    w("guids " + " ".join(hex(x) for x in guids))
    while time.time() - t0 < secs:
        obj = xmem.map_entries(g, 0x8365808C); hm = xmem.map_entries(g, 0x83656558)
        hp = H.update()
        parts = [f"t {time.time()-t0:.2f} her {tuple(round(c,1) for c in hp) if hp else None}"]
        for k in guids:
            tr = obj.get(k); tp = struct.unpack(">3f", g.read(tr, 12)) if tr else None
            sa = H.state_addr(k); sp = struct.unpack(">3f", g.read(sa, 12)) if sa else None
            a = hm.get(k); hv = struct.unpack(">2f", g.read(a, 8)) if a else None
            dead = None; lvl = None
            if tr:
                fb = g.read(tr + 0x1A, 1); dead = (fb[0] & 0x80) >> 7 if fb else None
                xb = struct.unpack(">H", g.read(tr + 0x18, 2) or bytes(2))[0] >> 4
                pool = H._u32(0x83134224)
                if pool and tp:
                    blk = pool + xb * 0x1F0
                    cnt = H._u32(blk + 0x1E4); ids = struct.unpack(f">{min(cnt,0x78)}I", g.read(blk, 4 * min(cnt, 0x78)) or b"") if cnt else ()
                    slot = ids.index(k) if k in ids else None
                    lvl = (g.read(blk + 0x80 + slot, 1) or bytes([255]))[0] if slot is not None else f"not in block {xb}"
            cell = (32 * math.floor((tp[2] + 4096) / 256) + math.floor((tp[0] + 4096) / 256)) if tp else None
            parts.append(f"{k:#x}: obj {'yes' if tr else 'NO'} {tuple(round(c,2) for c in tp) if tp else ''} dead-bit {dead} cell {cell} level {lvl} state {tuple(round(c,2) for c in sp) if sp else None} health {hv}")
        line = " | ".join(parts)
        q = g.read(0x8357F9B8, 0x40); grid = bytes((g.read(0x83134224 + c * 0x1F0 + 0x80, 1) or b"\xff")[0] for c in range(64))
        if q != last.get("q"): w(f"t {time.time()-t0:.2f} removal queue 0x8357F9B8: " + q.hex()); last["q"] = q
        if grid != last.get("g"): w(f"t {time.time()-t0:.2f} grid level bytes cells 0-63: " + grid.hex()); last["g"] = grid
        if line.split(" | ", 1)[1:] != last.get("l"): w(line); last["l"] = line.split(" | ", 1)[1:]
        time.sleep(0.5)
