#!/bin/sh
# settle4: runs after rpg7 finishes (coordinator 17:35Z go). Reads only + mission-skip pokes + GodMode.
. /c/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad/lib.sh
[ "$1" = go ] || { echo "settle4: needs go"; exit 1; }
until ! ps -ef | grep -v grep | grep -q "rpg7.sh"; do sleep 10; done
one settle4 settle4 "$PY settle4probe.py Z:/mercs2/scratch/xenia-compare/settle4 900 > $D/settle4-probe.out 2>&1 &"
