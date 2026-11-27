#!/bin/sh
# rpg3: needs "go" argument AND scratchpad/rpg3.go. Two-stage poke then the fight.
. /c/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad/lib.sh
[ "$1" = go ] && [ -f $S/rpg3.go ] || { echo "rpg3: needs go argument and rpg3.go file"; exit 1; }
rm -f $S/rpg3.go
one rpg3 rpg3 "$PY hpfast.py Z:/mercs2/scratch/xenia-compare/rpg3/hp.csv -3270 -2621 600 > $D/rpg3-hp.out 2>&1 & (sleep 2; $PY tpoke2.py Z:/mercs2/scratch/xenia-compare/rpg3/poke.txt -3255 105 -2621 15 > $D/rpg3-poke.out 2>&1) &"
