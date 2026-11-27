#!/bin/sh
# settle1 go: crates-settle (crates-settle-ask.md). Manual start only. Writes: mission-skip pokes + GodMode (as crates2) only.
. /c/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad/lib.sh
[ "$1" = go ] || { echo "settle1: needs go"; exit 1; }
one settle1 settle1 "$PY settleprobe.py Z:/mercs2/scratch/xenia-compare/settle1/settle.txt 900 > $D/settle1-probe.out 2>&1 &"
