"""Read-only physics chain (vehicle thread, 2026-11-19): guid -> store 0x836571D4 (lookup 0x823DAF68), record +0 = P (car vtable
0x820C8560). Bodies: list P+0x4C, count P+0x50, indirect when P+0x54 != 0; wrapper +0x38 = rigid body B; B+0x1A0 linear velocity,
B+0x1B0 angular. Wheels: P+0x158 -> C; count C+0x208; wheel i = u32(C+0x54+4i); float +0x0 == 0.4 touching; u32 +0x34 contacts.
The store's table class is not checked offline, so the lookup tries both pool shapes and keeps the one whose record holds P."""
import struct, math
import xmem
STORE, CAR_VT = 0x836571D4, 0x820C8560
def u32(g, a):
    b = g.read(a, 4) if a else None
    return struct.unpack(">I", b)[0] if b and len(b) == 4 else 0
def f3(g, a):
    b = g.read(a, 12)
    return struct.unpack(">3f", b) if b and len(b) == 12 else None
def ok_ptr(p): return 0x40000000 <= p < 0xC0000000 and p & 3 == 0
def lookup(g, H, guid):
    """(record address, P, how). Tries the keyed-pool class (map_entries) and the state-table class (map at +0x34)."""
    tries = []
    try: tries.append(("keyed", xmem.map_entries(g, STORE).get(guid)))
    except Exception: pass
    try: tries.append(("table", H.state_addr(guid, STORE)))
    except Exception: pass
    for how, rec in tries:
        if rec:
            P = u32(g, rec)
            if ok_ptr(P): return rec, P, how
    return None, None, None
def bodies(g, P):
    """[(wrapper, B)] under both readings of the list; the caller logs which gave sane velocities."""
    lst, n, ind = u32(g, P + 0x4C), u32(g, P + 0x50), u32(g, P + 0x54)
    out = []
    if not ok_ptr(lst) or not 0 < n <= 64: return lst, n, ind, out
    for i in range(n):
        if ind:   # +0x54 != 0: the list holds pointers to wrappers
            w = u32(g, lst + 4 * i)
        else:     # +0x54 == 0: the wrapper is stored inline at the list address (phys1: u32(lst) was the wrapper's vtable)
            if i: break  # inline stride for more than one body is unknown
            w = lst
        B = u32(g, w + 0x38) if ok_ptr(w) else 0
        out.append((w, B))
    return lst, n, ind, out
def body_state(g, B):
    v, wv = f3(g, B + 0x1A0), f3(g, B + 0x1B0)
    return v, wv
def wheels(g, P):
    C = u32(g, P + 0x158)
    if not ok_ptr(C): return C, []
    n = u32(g, C + 0x208); ws = []
    for i in range(min(n, 8)):
        w = u32(g, C + 0x54 + 4 * i)
        if ok_ptr(w):
            b = g.read(w, 0x38)
            if b and len(b) == 0x38: ws.append((struct.unpack_from(">f", b, 0)[0], struct.unpack_from(">I", b, 0x34)[0]))
    return C, ws
