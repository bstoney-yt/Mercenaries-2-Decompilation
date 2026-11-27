#!/bin/sh
# settle3: crates-settle retry, 60-80 m stop. Starts only with the "go" argument AND the go file scratchpad/settle3.go (created on the
# coordinator's GPU go). Writes: mission-skip pokes + GodMode only.
. /c/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad/lib.sh
[ "$1" = go ] && [ -f $S/settle3.go ] || { echo "settle3: needs go argument and settle3.go file"; exit 1; }
rm -f $S/settle3.go
one settle3 settle3 "$PY settle3probe.py Z:/mercs2/scratch/xenia-compare/settle3/settle.txt 900 > $D/settle3-probe.out 2>&1 &"
