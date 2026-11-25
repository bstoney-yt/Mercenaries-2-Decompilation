#!/bin/sh
# rpg6: needs "go" argument AND scratchpad/rpg6.go.
. /c/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad/lib.sh
[ "$1" = go ] && [ -f $S/rpg6.go ] || { echo "rpg6: needs go argument and rpg6.go file"; exit 1; }
rm -f $S/rpg6.go
one rpg6 rpg6 "$PY hpfast.py Z:/mercs2/scratch/xenia-compare/rpg6/hp.csv -3270 -2621 1800 > $D/rpg6-hp.out 2>&1 & (sleep 2; $PY hopchain.py Z:/mercs2/scratch/xenia-compare/rpg6/hops.txt -3255 -2621 130 25 > $D/rpg6-hops.out 2>&1) &"
