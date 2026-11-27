#!/bin/sh
# wreck1 go: the Phoenix wreck's bodies (wreck-bodies-xenia-ask.md). Manual start only. Writes: only the route's mission-skip pokes
# and the death2 route's existing shootat weaken (car health set to 1), as phys8a.
. /c/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad/lib.sh
[ "$1" = go ] || { echo "wreck1: needs go"; exit 1; }
for r in wreck1a wreck1b; do
  one $r death2 "$PY wreckprobe.py Z:/mercs2/scratch/xenia-compare/$r/wreck.txt 2653.8 -996.7 150 > $D/$r-wreck.out 2>&1 &" || exit 1
done
