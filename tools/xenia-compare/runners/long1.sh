#!/bin/sh
# short.sh: short GPU window 2026-11-19: #728 body probe (death2 route) then #673 ragdoll read (wrjen route).
S=/c/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad
D=/z/mercs2/scratch/xenia-compare
PY="uv run -q --no-project --with numpy python -I"
one() { # name route probes-cmd
  n=$1; r=$2
  until [ "$(tasklist | grep -ciE 'm2-alpha-xenia')" = 0 ]; do sleep 5; done
  rm -rf $D/$n
  pwsh -NoProfile -ExecutionPolicy Bypass -File "$(cygpath -w $S/run1.ps1)" -Name $n -Route $r > $D/$n-run.out 2>&1 &
  until grep -aqE "poke 0x82b7c953|xenia closed|Traceback" $D/$n/route.log 2>/dev/null; do sleep 2; done
  if ! grep -aq "poke 0x82b7c953 = 0: ok, reads back" $D/$n/route.log; then
    echo "ATTACH FAILED $n"; grep -a poke $D/$n/route.log
    pwsh -NoProfile -Command "Get-Process m2-alpha-xenia -ErrorAction SilentlyContinue | Stop-Process"; return 1; fi
  echo "attach ok $n"; (cd $S && $PY readfloats.py "title $n") >> $D/$n-floats.out 2>&1
  until grep -aqE "walkcam: at|walkcam .*TIMED|xenia closed|Traceback" $D/$n/route.log 2>/dev/null; do sleep 1; done
  shift 2; (cd $S && eval "$@")
  until grep -aqE "xenia closed|Traceback" $D/$n/route.log 2>/dev/null; do sleep 3; done
  sleep 5; echo "done $n"
}
one phys1 death2 "$PY bodyprobe2.py Z:/mercs2/scratch/xenia-compare/phys1/body.txt 2653.8 -996.7 120 > $D/phys1-body.out 2>&1 &" || exit 1
one rag3 rag1 "$PY ragprobe.py Z:/mercs2/scratch/xenia-compare/rag3/rag.txt 2653.8 -996.7 150 > $D/rag3-rag.out 2>&1 &"
