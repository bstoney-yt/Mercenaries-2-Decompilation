#!/bin/sh
# trailers1 go: Fuel Trailers 0xF0309 / 0xF030B watch. NEEDS the position poke (no start within ~1 km); blocked until the poke fix
# (character-controller position) is in tpoke.py. Manual start only.
. /c/Users/dev/AppData/Local/Temp/claude/b0147d3e513e77ed/61b02c61-65f9-431d-8481-4f2b8f31b793/scratchpad/lib.sh
[ "$1" = go ] || { echo "trailers1: needs go"; exit 1; }
one trailers1 trailers1 "$PY dockprobe.py Z:/mercs2/scratch/xenia-compare/trailers1/trailers.txt 300 F0309 F030B > $D/trailers1-probe.out 2>&1 & (sleep 2; $PY tpoke.py Z:/mercs2/scratch/xenia-compare/trailers1/poke.txt 1250 -26.0 393 10 > $D/trailers1-poke.out 2>&1) &"
