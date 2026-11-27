#!/bin/sh
# m151b: after tank3; only if tank3's poke stuck (her record within 6 m of the poke point after 10 s).
. /c/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad/lib.sh
until ! ps -ef | grep -v grep | grep -q "tank3.sh\|nextwin.sh"; do sleep 10; done
if ! (cd $S && python stick.py Z:/mercs2/scratch/xenia-compare/tank3/poke.txt -2618 405 6); then echo "SKIP m151b: tank3 poke did not stick"; exit 0; fi
one m151b m151b "$PY tankfall.py Z:/mercs2/scratch/xenia-compare/m151b/m151.txt 133BA2 400 > $D/m151b-probe.out 2>&1 & (sleep 2; $PY tpoke.py Z:/mercs2/scratch/xenia-compare/m151b/poke.txt 1041.6 39.5 168.0 10 > $D/m151b-poke.out 2>&1) &"
