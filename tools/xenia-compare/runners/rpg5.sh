#!/bin/sh
# rpg5: needs "go" argument AND scratchpad/rpg5.go.
. /c/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad/lib.sh
[ "$1" = go ] && [ -f $S/rpg5.go ] || { echo "rpg5: needs go argument and rpg5.go file"; exit 1; }
rm -f $S/rpg5.go
one rpg5 rpg5 "$PY hpfast.py Z:/mercs2/scratch/xenia-compare/rpg5/hp.csv -3270 -2621 900 > $D/rpg5-hp.out 2>&1 & (sleep 2; $PY tpoke4.py Z:/mercs2/scratch/xenia-compare/rpg5/poke.txt -3255 105 -2621 > $D/rpg5-poke.out 2>&1) &"
