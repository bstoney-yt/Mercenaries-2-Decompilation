#!/bin/sh
# nextwin.sh: next GPU window, back to back (Jennifer). Each block stops the chain if the pokes do not read back.
. /c/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad/lib.sh
one phys7 death2 "$PY massprobe.py Z:/mercs2/scratch/xenia-compare/phys7/mass.txt 2653.8 -996.7 150 > $D/phys7-mass.out 2>&1 &" || exit 1
one tank2 tank2 "$PY tankfall.py Z:/mercs2/scratch/xenia-compare/tank2/tank.txt B4BC2 900 > $D/tank2-probe.out 2>&1 & $PY camprobe.py Z:/mercs2/scratch/xenia-compare/tank2/cam.txt 900 > $D/tank2-cam.out 2>&1 & $PY stepdt.py Z:/mercs2/scratch/xenia-compare/tank2/stepdt.txt 20 > /dev/null 2>&1 &" || exit 1
one pmc32b pmc32b "$PY camprobe.py Z:/mercs2/scratch/xenia-compare/pmc32b/cam.txt 300 > $D/pmc32b-cam.out 2>&1 & $PY glprobe.py Z:/mercs2/scratch/xenia-compare/pmc32b 300 > $D/pmc32b-gl.out 2>&1 &" || exit 1
one m151 m151 "$PY tankfall.py Z:/mercs2/scratch/xenia-compare/m151/m151.txt 133BA2 1200 > $D/m151-probe.out 2>&1 &" || exit 1
one rpg1 rpg1 "$PY hpfast.py Z:/mercs2/scratch/xenia-compare/rpg1/hp.csv -3270 -2621 600 > $D/rpg1-hp.out 2>&1 &" || exit 1
one phys7b death2 "$PY massprobe.py Z:/mercs2/scratch/xenia-compare/phys7b/mass.txt 2653.8 -996.7 150 > $D/phys7b-mass.out 2>&1 &" || exit 1
one phys7c death2 "$PY massprobe.py Z:/mercs2/scratch/xenia-compare/phys7c/mass.txt 2653.8 -996.7 150 > $D/phys7c-mass.out 2>&1 &" || exit 1
one phys7d death2 "$PY massprobe.py Z:/mercs2/scratch/xenia-compare/phys7d/mass.txt 2653.8 -996.7 150 > $D/phys7d-mass.out 2>&1 &" || exit 1
one boat1 boat1 "$PY boatprobe.py Z:/mercs2/scratch/xenia-compare/boat1/boat.txt 900 FA47B D5FC4 D5FC7 D5FC6 > $D/boat1-probe.out 2>&1 &"
