#!/bin/sh
# win4.sh go: alljob1 (no poke) then copylog (one short proven poke). Needs "go" argument AND scratchpad/win4.go.
. /c/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad/lib.sh
[ "$1" = go ] && [ -f $S/win4.go ] || { echo "win4: needs go argument and win4.go file"; exit 1; }
rm -f $S/win4.go
one alljob1 alljob1 "$PY nearrpg.py Z:/mercs2/scratch/xenia-compare/alljob1/near.txt 600 > $D/alljob1-near.out 2>&1 &"
one copylog copylog "$PY copylog.py Z:/mercs2/scratch/xenia-compare/copylog/copies.txt > $D/copylog-copies.out 2>&1 &"
echo "win4 done"
