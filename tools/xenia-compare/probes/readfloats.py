# readfloats.py <label>: read-only, prints the four ragdoll death-exit floats 0x82DE8894..0x82DE88A0 (image: 10.0, 0.0, 10.0, 0.0).
import sys, struct, subprocess
sys.path.insert(0, r"C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare")
import xmem
out = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True)
pid = next(int(l.split('","')[1]) for l in out.splitlines() if "xenia" in l.lower())
b = xmem.Guest(pid).read(0x82DE8894, 16)
print(sys.argv[1], struct.unpack(">4f", b) if b and len(b) == 16 else None, "(image 10.0, 0.0, 10.0, 0.0)")
b2 = xmem.Guest(pid).read(0x8380EB38, 8)
print(sys.argv[1], "launch rand scales 0x8380EB38/3C", struct.unpack(">2f", b2) if b2 and len(b2) == 8 else None, "(image 0.0, 0.0)")
