#!/bin/sh
# rest.sh go: glr3 then phys8a/b/c (no position pokes), after parked1 finishes. Manual start only.
. /c/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad/lib.sh
[ "$1" = go ] || { echo "rest: needs go"; exit 1; }
until ! ps -ef | grep -v grep | grep -q "parked1.sh"; do sleep 10; done
one glr3 glr3 "$PY glprobe2.py Z:/mercs2/scratch/xenia-compare/glr3 300 > $D/glr3-gl.out 2>&1 & $PY ordlog.py Z:/mercs2/scratch/xenia-compare/glr3/ord.csv 300 > $D/glr3-ord.out 2>&1 &" || exit 1
for r in phys8a phys8b phys8c; do
  one $r death2 "$PY phys8probe.py Z:/mercs2/scratch/xenia-compare/$r/p8.txt 2653.8 -996.7 150 > $D/$r-p8.out 2>&1 &" || exit 1
done
