# Seconds per training step of several switch settings (one hull-fit step, 40 steps, the last 20 timed from the log).
# usage: time_variants.sh PACK [NAME:VAR=VALUE,VAR=VALUE ...]   (run from a scratch directory)
PACK=${1:?pack}; shift; HERE=$(cd "$(dirname "$0")" && pwd)
[ $# -gt 0 ] || set -- "default:" "whole-ray-128:HULL_BAND=0,NEUS_COARSE=64,NEUS_ROUND=16" "reference-width:HULL_BAND=0,NEUS_COARSE=64,NEUS_ROUND=16,WIDTH=256,GEO=256,GRADIENT=separate"
for v in "$@"; do
  name=${v%%:*}; vars=$(echo "${v#*:}" | tr ',' ' ')
  # HULL=1: a one-step hull fit, enough to build the hull grid that the band needs.
  env HULL=1 $vars STOP=41 LOG_EVERY=20 python "$HERE/neus_facto_mlx.py" "$PACK" timing-$name > timing-$name.log 2>&1
  a=$(grep "step 20:" timing-$name.log | sed 's/^\[ *\([0-9]*\)s.*/\1/'); b=$(grep "step 40:" timing-$name.log | sed 's/^\[ *\([0-9]*\)s.*/\1/')
  echo "$name: $(echo "scale=2; ($b - $a) / 20" | bc) s/step, $(grep 'step 40:' timing-$name.log | grep -o 'peak.*')"
  rm -rf timing-$name
done
