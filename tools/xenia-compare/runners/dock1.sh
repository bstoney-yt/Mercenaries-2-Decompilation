#!/bin/sh
# dock1: MANUAL START ONLY (on the coordinator's GPU go), and only if a tank3 retry showed the poke sticks.
. /c/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad/lib.sh
[ "$1" = go ] || { echo "dock1: needs 'go' argument (manual start only)"; exit 1; }
(cd $S && python stick.py Z:/mercs2/scratch/xenia-compare/tank3/poke.txt -2618 405 6) || { echo "SKIP dock1: poke not shown to stick"; exit 0; }
one dock1 dock1 "$PY dockprobe.py Z:/mercs2/scratch/xenia-compare/dock1/dock.txt 300 94B0E 94B0F 94B10 94B12 94B13 8AB91 B19CF B19D0 > $D/dock1-probe.out 2>&1 & (sleep 2; $PY tpoke.py Z:/mercs2/scratch/xenia-compare/dock1/poke.txt 1755 -15.0 470 10 > $D/dock1-poke.out 2>&1) &"
