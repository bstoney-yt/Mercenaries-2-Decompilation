#!/bin/sh
# rpg7: needs "go" argument AND scratchpad/rpg7.go.
. /c/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad/lib.sh
[ "$1" = go ] && [ -f $S/rpg7.go ] || { echo "rpg7: needs go argument and rpg7.go file"; exit 1; }
rm -f $S/rpg7.go
one rpg7 rpg7 "$PY hpfast.py Z:/mercs2/scratch/xenia-compare/rpg7/hp.csv -3270 -2621 1800 > $D/rpg7-hp.out 2>&1 & (sleep 2; $PY hopchain2.py Z:/mercs2/scratch/xenia-compare/rpg7/hops.txt -3255 -2621 130 15 > $D/rpg7-hops.out 2>&1) &"
