#!/bin/sh
# tank3b: after the win3 chain. Her proxy poke (tpoke) then the tank lift (tankpoke). Needs "go".
. /c/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad/lib.sh
[ "$1" = go ] || { echo "tank3b: needs go"; exit 1; }
until ! ps -ef | grep -v grep | grep -q "win3.sh"; do sleep 10; done
one tank3b tank3b "(sleep 2; $PY tpoke.py Z:/mercs2/scratch/xenia-compare/tank3b/poke.txt -2618 -15.0 405 10 > $D/tank3b-poke.out 2>&1; sleep 15; $PY tankpoke.py Z:/mercs2/scratch/xenia-compare/tank3b/tankpoke.txt B4BC2 10 8 > $D/tank3b-tankpoke.out 2>&1) &"
