#!/bin/sh
# rpg4: needs "go" argument AND scratchpad/rpg4.go.
. /c/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad/lib.sh
[ "$1" = go ] && [ -f $S/rpg4.go ] || { echo "rpg4: needs go argument and rpg4.go file"; exit 1; }
rm -f $S/rpg4.go
one rpg4 rpg4 "$PY hpfast.py Z:/mercs2/scratch/xenia-compare/rpg4/hp.csv -3270 -2621 900 > $D/rpg4-hp.out 2>&1 & (sleep 2; $PY tpoke3.py Z:/mercs2/scratch/xenia-compare/rpg4/poke.txt -3255 105 -2621 > $D/rpg4-poke.out 2>&1) &"
