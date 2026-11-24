#!/bin/sh
# chain.sh <name...>: run routes back to back (GPU window granted), no gaps
S=/c/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad
D=/z/mercs2/scratch/xenia-compare
for n in "$@"; do
  until [ "$(tasklist | grep -ciE 'm2-alpha-xenia')" = 0 ]; do sleep 5; done
  rm -rf $D/$n
  r=$n; [ "$n" = tossb2 ] && r=tossb
  pwsh -NoProfile -ExecutionPolicy Bypass -File "$(cygpath -w $S/run1.ps1)" -Name $n -Route $r > $D/$n-run.out 2>&1
  echo done $n
done
