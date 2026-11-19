# blastpool.py <out.txt> <seconds>: read-only. Every entry that appears in the area-damage pool 0x8365650C
# (walked by 0x82292E30, which calls 0x82291B88 -> 0x82291640): its handle, the entry's first 0x40 bytes
# (+0x0C handle, +0x1C params pointer, +0x20/+0x24 sub-objects, +0x2C, +0x38 age, +0x3C flags), the params record's
# first 0x40 bytes as floats and words, the state-table position of its handle, and the hero's health/frame.
# Also every new state-table handle (diff of the map's keys) per poll.
import sys, struct, subprocess, time
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
import xmem
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
g = xmem.Guest(pid); H = xmem.Hero(g)
def state_keys():
    m = H.STATES + 0x34
    size, keys, buckets = H._u32(m + 8), H._u32(m + 0x10), H._u32(m + 0x14)
    kb, bb = g.read(keys, 4 * size), g.read(buckets, 4 * size)
    if not kb or not bb: return set()
    return {struct.unpack_from(">I", kb, e * 4)[0] for e in struct.unpack(f">{size}I", bb) if e != 0xFFFFFFFF and e < size}
seen = set(xmem.map_entries(g, 0x8365650C)); sk = state_keys()
hero = H.character(); ha = xmem.map_entries(g, 0x83656558).get(hero)
end = time.time() + float(sys.argv[2]); t0 = time.time()
with open(sys.argv[1], "w") as fh:
    fh.write(f"start: {len(seen)} area-damage entries, {len(sk)} state keys\n")
    while time.time() < end:
        hv = struct.unpack(">f", g.read(ha + 4, 4))[0] if ha else float("nan")
        sa = H.state_addr(hero); fc = H._u32(sa + 12) if sa else 0
        for k, a in xmem.map_entries(g, 0x8365650C).items():
            if k in seen: continue
            seen.add(k); sys.__dict__.setdefault("_ad", []).append(k)
            e = g.read(a, 0x40) or b""
            pp = struct.unpack_from(">I", e, 0x1C)[0] if len(e) >= 0x20 else 0
            prm = g.read(pp, 0x40) if pp else None
            ps = H.state_addr(k); pos = struct.unpack(">3f", g.read(ps, 12)) if ps else None
            # the emitter's own transform record: 0x82187750 resolves the entry's (table 0x83658074, handle) through
            # the keyed pool at 0x8365808C; 0x82187950 reads position floats at +0 and a packed quaternion at +0xC
            tr = xmem.map_entries(g, 0x8365808C).get(k)
            tpos = struct.unpack(">3f", g.read(tr, 12)) if tr else None
            tq = struct.unpack(">4h", g.read(tr + 0xC, 8)) if tr else None
            fh.write(f"  transform record {tr and hex(tr)} pos {tpos} quat-shorts {tq}" + chr(10))
            fh.write(f"t {time.time()-t0:.4f} frame {fc} hero_hp {hv:.2f} NEW area-damage {k:#x} entry {a:#x} pos {pos}\n")
            fh.write("  entry words " + " ".join(f"{w:08x}" for w in struct.unpack(f">{len(e)//4}I", e)) + "\n")
            if prm:
                fh.write("  params@%#x floats " % pp + " ".join(f"{f:.4g}" for f in struct.unpack(">16f", prm)) + "\n")
                fh.write("  params words " + " ".join(f"{w:08x}" for w in struct.unpack(">16I", prm)) + "\n")
        if len(sys.argv) > 3:
            ca = H.state_addr(int(sys.argv[3], 16)); cb = g.read(ca, 32) if ca else None
            cpos = struct.unpack(">3fI4f", cb) if cb and len(cb) == 32 else None
            if cpos != getattr(sys, "_lastcar", None):
                sys._lastcar = cpos
                fh.write(f"t {time.time()-t0:.4f} frame {fc} car pos {cpos and cpos[0:3]} counter {cpos and cpos[3]} quat {cpos and cpos[4:8]}" + chr(10))
        for kk in list(getattr(sys, "_ad", [])):
            tr = xmem.map_entries(g, 0x8365808C).get(kk)
            if tr:
                tp = struct.unpack(">3f", g.read(tr, 12)); tq = struct.unpack(">4h", g.read(tr + 0xC, 8))
                if getattr(sys, "_adlast", {}).get(kk) != (tp, tq):
                    sys.__dict__.setdefault("_adlast", {})[kk] = (tp, tq)
                    fh.write(f"t {time.time()-t0:.4f} frame {fc} emitter {kk:#x} pos {tp} quat-shorts {tq}" + chr(10))
        nk = state_keys()
        new = nk - sk
        if new:
            fh.write(f"t {time.time()-t0:.4f} frame {fc} hero_hp {hv:.2f} new state handles: " + " ".join(hex(x) for x in sorted(new)) + "\n")
        sk = nk; fh.flush(); time.sleep(0.001)
