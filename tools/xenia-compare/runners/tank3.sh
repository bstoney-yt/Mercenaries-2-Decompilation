#!/bin/sh
# tank3: runs after nextwin.sh (wait for it), its own block.
. /c/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad/lib.sh
until ! ps -ef | grep -v grep | grep -q nextwin.sh; do sleep 10; done
one tank3 tank3 "$PY tankfall.py Z:/mercs2/scratch/xenia-compare/tank3/tank.txt B4BC2 600 > $D/tank3-probe.out 2>&1 & $PY camprobe.py Z:/mercs2/scratch/xenia-compare/tank3/cam.txt 300 > $D/tank3-cam.out 2>&1 & $PY stepdt.py Z:/mercs2/scratch/xenia-compare/tank3/stepdt.txt 20 > /dev/null 2>&1 & (sleep 2; $PY tpoke.py Z:/mercs2/scratch/xenia-compare/tank3/poke.txt -2618 -15.0 405 10 > $D/tank3-poke.out 2>&1) &"
