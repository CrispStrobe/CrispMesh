# OpenMVG Sceaux photo control: read-only post-run validation

Status: validator implemented and tested on synthetic fixtures, then run once
read-only against the completed `-002` output. The preserved
`openmvg-sceaux-photo-sfm-001` receipt is stopped at the feature stage. The
validator accepts only a completed v2 receipt at the reviewed `-002` path.
No OpenMVG command or reference comparison was run for this work.

The pinned v2.1 source at commit
`01193a245ee3c36458e650b1cf4402caad8983ef` shows that `main_SfM.cpp`
writes `sparse/sfm_data.bin` with cereal, `sparse/cloud_and_poses.ply` with
binary little-endian double XYZ plus RGB, and
`sparse/SfMReconstruction_Report.html`. In `sfm_data_io_ply.hpp`, green points
are posed-view centers, white points are landmarks, blue points are pose
priors, and red points are control points. In `sfm_report.cpp`, the first
`residualCount` increments once per observation, so `#residuals` counts
**observations**. The later local counter counts both scalar components for
RMSE and histogram construction. Each view's residual statistics are computed
over both absolute pixel-coordinate components. Each per-view row
has observation statistics only when its pose and intrinsic are defined.
Its `Basename` field uses `stlplus::basename_part` and contains an extensionless
stem (`00010`, ..., `00000`), which the validator maps to the exact sealed
`.jpg` inventory; duplicate or malformed stems fail closed.

The [postcheck](../scripts/classical_backend/openmvg_sceaux_postcheck.py)
accepts only a completed v2 five-stage receipt at the `-002` root and verifies the receipt's
SHA-256/byte seals for the model, PLY, and report before and after reading.
It requires the exact 11 `00000.jpg`–`00010.jpg` per-view report rows, 11
poses, positive observations in every view, nonzero tracks, count agreement
between all observations and `#residuals`, finite nonnegative report
statistics, and finite PLY coordinates. Its PLY camera and landmark counts
must match report poses and tracks; pose-prior/control-point colors are
rejected for this image-only control. The output JSON contains per-view
observation and residual summaries, scene RMSE, PLY coordinate bounds, and
all three artifact hashes. The four-pixel scene-RMSE and worst-view-median
flag is a **diagnostic**, not a shape-quality or reference-accuracy claim.

The cereal binary remains opaque: no reviewed decoder or conversion CLI is
present in the five-target build. Consequently this checker does **not**
independently verify camera rotations, intrinsics, track membership, or
recomputed reprojections. It reports `validated_partial_model` and lists
these limits explicitly. A malformed or incomplete artifact fails closed;
the current stopped run cannot pass. No upstream camera, mesh, or reference
is opened, and none enters reconstruction.

After a separately completed photo run, an operator can invoke:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 \
  -m scripts.classical_backend.openmvg_sceaux_postcheck \
  --output /Volumes/backups/code/crispmesh-data/openmvg-sceaux-photo-sfm-002
```

The command prints JSON to stdout and writes nothing. It needs only the
Python standard library. Its [synthetic tests](../scripts/classical_backend/test_openmvg_sceaux_postcheck.py)
cover healthy 11-view data, missing/empty observations, nonfinite PLY data,
duplicate/malformed report stems, count disagreement, artifact-seal changes,
incomplete receipts, and the
diagnostic reprojection flag.

The read-only `-002` invocation returned `validated_partial_model`: 11/11
posed views with positive observations, 6,048 tracks, 19,807 observations,
11 finite green camera-center points and 6,048 finite white landmark points.
The report's per-axis scene RMSE is 0.392261 pixels; the worst per-view median
is 0.171747 pixels, so the diagnostic four-pixel flag is true. The PLY/report
counts and sealed artifact hashes agree. This establishes internal
photo-only sparse-output consistency, not camera accuracy or reconstruction
quality against a reference. The binary model remains unparsed.
