#!/bin/sh
# rpg8: needs "go" argument AND scratchpad/rpg8.go.
. /c/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad/lib.sh
[ "$1" = go ] && [ -f $S/rpg8.go ] || { echo "rpg8: needs go argument and rpg8.go file"; exit 1; }
rm -f $S/rpg8.go
one rpg8 rpg8 "$PY hpfast.py Z:/mercs2/scratch/xenia-compare/rpg8/hp.csv -3270 -2621 1800 > $D/rpg8-hp.out 2>&1 & (sleep 2; $PY hopchain3.py Z:/mercs2/scratch/xenia-compare/rpg8/hops.txt -3255 -2621 130 15 > $D/rpg8-hops.out 2>&1) &"
