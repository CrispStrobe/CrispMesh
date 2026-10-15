#!/bin/bash
# Scores an existing case (work/CASE/run/mesh/mesh.stl) and makes its camera report and sheet.
scripts="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$scripts/../.." && pwd)"
# Data (models/, bin/crispmesh-dense, work/) lives outside git; override with GSO_EVAL_DIR.
here="${GSO_EVAL_DIR:-$repo/.local-tools/gso-eval}"
py="${PYTHON:-python}"
work=$here/work/$1
rm -rf "$work/score"
PYTHONPATH="$repo" "$py" "$repo/scripts/turntable_mesh/scan_evaluate.py" --mesh "$work/run/mesh/mesh.stl" \
  --reference "$work/capture/reference.ply" --output "$work/score" --no-platform 2>&1 | tail -4
"$py" "$scripts/camera_error.py" "$work" | tr -d '\n '; echo
"$py" "$scripts/compare_sheet.py" "$work"
