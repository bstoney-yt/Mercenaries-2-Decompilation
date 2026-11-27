#!/bin/sh
# parked1 + hold1 (Phoenix): after m151b (or its skip).
. /c/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad/lib.sh
[ "$1" = go ] || { echo "parked1: needs go argument (manual start only)"; exit 1; }
one parked1 parked1 "$PY carhold.py Z:/mercs2/scratch/xenia-compare/parked1/hold.txt 900 10D5D8 FA056 > $D/parked1-hold.out 2>&1 & $PY tankfall.py Z:/mercs2/scratch/xenia-compare/parked1/elgrande.txt FA056 900 > $D/parked1-tank.out 2>&1 &"
