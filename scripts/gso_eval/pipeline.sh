#!/bin/bash
# One GSO evaluation case: render a turntable capture of models/MODEL, run the default command from
# the photos, score with scan_evaluate --no-platform against the placed mesh.
#   pipeline.sh CASE MODEL [render options...]     (results in work/CASE)
# Defaults: 72 views at 20 degrees, 1749 x 1155, the 3DLF lens, blur 1.0 px, noise 0.01.
# MASKS=exact uses the renderer's silhouettes (--masks import:), RUN_EXTRA adds run options,
# KEEP=1 keeps photos and run intermediates. Environment: PYTHON (with numpy, scipy, opencv),
# CRISPMESH_DENSE (release binary, default $GSO_EVAL_DIR/bin/crispmesh-dense), MIN_FREE_GIB (9).
# Workflow: fetch.sh MODEL... ; pipeline.sh or batch.sh < list ; summarize.py ;
# prepare_dataset.py for an example-objects dataset. Results: docs/OTHER-IMAGE-SETS.md.
set -u
case_name=$1; model=$2; shift 2
scripts="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$scripts/../.." && pwd)"
# Data (models/, bin/crispmesh-dense, work/) lives outside git; override with GSO_EVAL_DIR.
here="${GSO_EVAL_DIR:-$repo/.local-tools/gso-eval}"
py="${PYTHON:-python}"
bin="${CRISPMESH_DENSE:-$here/bin/crispmesh-dense}"
lens="${LENS:-$repo/scripts/turntable_mesh/calibrations/3dlf-pro.json}"
work=$here/work/$case_name
mkdir -p "$work" || exit 1
log=$work/log.txt
: > "$log"
free=$(df -g "$here" | awk 'NR==2{print $4}')
[ "$free" -lt "${MIN_FREE_GIB:-9}" ] && { echo "only ${free} GiB free; stop" | tee -a "$log"; exit 2; }
t0=$(date +%s)
"$bin" render --mesh "$here/models/$model/meshes/model.obj" --output "$work/capture" --calibration "$lens" \
  --views 72 --elevation 20 --blur 1.0 --noise 0.01 --threads 4 "$@" >> "$log" 2>&1 || { echo "render failed" >> "$log"; exit 1; }
t1=$(date +%s)
"$bin" run --photos "$work/capture/photos" --calibration "$work/capture/lens.json" --output "$work/run" --threads 4 \
  ${RUN_EXTRA:-} ${MASKS:+--masks import:$work/capture/masks} >> "$log" 2>&1
status=$?
t2=$(date +%s)
echo "render_s=$((t1 - t0)) run_s=$((t2 - t1)) run_status=$status" >> "$log"
stl=$(ls "$work"/run/mesh/mesh.stl 2>/dev/null | head -1)
if [ -n "$stl" ]; then
  PYTHONPATH="$repo" "$py" "$repo/scripts/turntable_mesh/scan_evaluate.py" --mesh "$stl" --reference "$work/capture/reference.ply" \
    --output "$work/score" --no-platform >> "$log" 2>&1
fi
if [ -f "$work/score/result.json" ]; then
  "$py" "$scripts/camera_error.py" "$work" >> "$log" 2>&1
  "$py" "$scripts/compare_sheet.py" "$work" >> "$log" 2>&1
fi
if [ -z "${KEEP:-}" ]; then
  # Keep the mesh, scores, sheets and reports; renders are deterministic and can be made again.
  bash "$scripts/clean.sh" "$case_name"
fi
echo "done total_s=$(( $(date +%s) - t0 ))" >> "$log"
