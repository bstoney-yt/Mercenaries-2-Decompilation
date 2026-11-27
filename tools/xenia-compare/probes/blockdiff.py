# blockdiff.py <label>: read-only, the tunables block 0x82DE87F8..0x82DE88BC (50 words) live vs the image values in
# C:\Claude\scratch\ragdoll\block.txt; prints every word that differs (address, image, live) and a one-line digest of all live words.
import sys, struct, subprocess, hashlib
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
import xmem
img = {}
for l in open(r"C:\Claude\scratch\ragdoll\block.txt"):
    p = l.split()
    if len(p) >= 2 and p[1].startswith("w="): img[int(p[0], 16)] = int(p[1][2:], 16)
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
N = (0x82DE894C - 0x82DE87F8) // 4 + 1  # through 0x82DE894C inclusive (86 words)
b = xmem.Guest(pid).read(0x82DE87F8, N * 4)
if not b or len(b) < N * 4: print(sys.argv[1], "READ FAILED"); sys.exit()
live = struct.unpack(f">{N}I", b); f = lambda w: struct.unpack(">f", struct.pack(">I", w))[0]
diffs = [(0x82DE87F8 + 4 * i, img.get(0x82DE87F8 + 4 * i), w) for i, w in enumerate(live) if img.get(0x82DE87F8 + 4 * i) != w]
print(sys.argv[1], f"words {len(live)} image {len(img)} differ {len(diffs)} digest {hashlib.md5(b).hexdigest()[:12]}")
for a, i, w in diffs: print(f"  {a:08X} image {i if i is None else hex(i)} ({'-' if i is None else f(i)}) live {w:#x} ({f(w)})")

names = {0x82DE88AC: "port KICK_SMOOTH 0.1", 0x82DE88B0: "port LEDGE_GRACE 0.25", 0x82DE88B8: "seats::CLEAR_TURNS start"}
print("  extension 0x82DE88BC..0x82DE894C, every word (address image live):")
for i in range((0x82DE88AC - 0x82DE87F8) // 4, N):
    a = 0x82DE87F8 + 4 * i; im = img.get(a); w = live[i]
    if a == 0x82DE88F0: print("  -- size-share table (PUSH_BY_SIZE) 0x82DE88F0..0x82DE894C, 12 pairs --")
    tag = "" if im == w else "   <== DIFFERS"
    print(f"  {a:08X} image {'-' if im is None else f(im)} live {f(w)}{tag} {names.get(a, '')}")
sz = [(f(live[(0x82DE88F0 - 0x82DE87F8) // 4 + 2 * k]), f(live[(0x82DE88F0 - 0x82DE87F8) // 4 + 2 * k + 1])) for k in range(12)]
szi = [(f(img[0x82DE88F0 + 8 * k]), f(img[0x82DE88F0 + 8 * k + 4])) for k in range(12)]
print("  size-share pairs live:", sz)
print("  size-share pairs image:", szi, "SAME" if sz == szi else "**SIZE-SHARE TABLE DIFFERS LIVE**")
