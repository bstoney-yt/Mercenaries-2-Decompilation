#!/bin/sh
# win2.sh go: next GPU window (Brendan: full approval for Xenia work, 2026-11-25 15:01Z; GPU go from the coordinator).
# Needs the "go" argument AND scratchpad/win2.go. Order: settle3 (reads only), tank3 (single-field phantom poke + stick test),
# then only if the poke sticks: trailers1, m151b, dock1, rpg2. Stops early if C: drops under 200 MB.
. /c/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad/lib.sh
[ "$1" = go ] && [ -f $S/win2.go ] || { echo "win2: needs go argument and win2.go file"; exit 1; }
rm -f $S/win2.go
disk() { f=$(df -m /c | tail -1 | awk '{print $4}'); [ "$f" -gt 200 ] || { echo "STOP: C: free $f MB"; exit 1; }; }
disk; touch $S/settle3.go; sh $S/settle3.sh go
disk; rm -f /z/mercs2/scratch/xenia-compare/tank3/poke.txt; sh $S/tank3.sh
if ! (cd $S && python stick.py Z:/mercs2/scratch/xenia-compare/tank3/poke.txt -2618 405 6); then echo "SKIP rest: tank3 poke did not stick"; exit 0; fi
disk; sh $S/trailers1.sh go
disk; sh $S/m151b.sh
disk; sh $S/dock1.sh go
disk; one rpg2 rpg2 "$PY hpfast.py Z:/mercs2/scratch/xenia-compare/rpg2/hp.csv -3270 -2621 600 > $D/rpg2-hp.out 2>&1 & (sleep 2; $PY tpoke.py Z:/mercs2/scratch/xenia-compare/rpg2/poke.txt -3255 107 -2621 10 > $D/rpg2-poke.out 2>&1) &"
echo "win2 done"
