# OpenMVG mustard two-arm camera comparison — posthoc review gate

This is an **evaluation-only, posthoc** comparison, not a reconstruction
input or model-selection loop. Both SfM runs and their JSON conversions are
already sealed. This worker has no calls to any SfM binary and reads Berkeley
metadata only after checking both export receipts/JSON/model hashes. Neither
Berkeley K nor NP5 poses, depth, scanner data, checkerboard corners, masks or
held-out photos were supplied to either OpenMVG arm.

## Frozen arms and fair intersection

`ADJUST_ALL` export -001 is bound by receipt SHA-256
`ab73c2b729adb8abd1f84d97f1908b26635cd8caa6d2d1756185d9a31a34dac3`
and JSON SHA-256
`c1e0ce4431245ca9392dd82b64cc8007d69e0c41a2e87cf78cec10f33ecc1124`;
its SfM model hash is
`dc11f9b3a74809ebc080260b360ff1dd0e6f9f9526ea151aac002ace75b2df8b`.
Fixed-intrinsic `NONE` export -003 is bound by receipt SHA-256
`ba5450e939b824f1c529b53e65538d4d38ab5bb3bd8bf7f073b38015e2b50088`,
JSON SHA-256
`a9f6b6aa9ce63e163a9f1a324768a2593fefcdf10e3f149dc61c4ccd5d6c1854`,
source SfM receipt SHA-256
`5445b512ac6d171e8097134d9c86d84e6428b1d958a3088593d6234e5bb1fa24`
and model SHA-256
`4147cd6ea9e0848dd97210701535371c100425125dc83df07a67482ebd8eca55`.
The existing ID-bound cereal parser verifies view `id_pose` → extrinsic key,
not array position. Both arms have exactly the same 46 registered TRAIN names;
`NP3_042.jpg` and `NP3_048.jpg` are absent. Scoring uses exactly those 46,
with no substitution or registration-count advantage for either arm.

## Source metadata and bounded execution

The previously cached official Berkeley mustard RGB-D archive is pinned by
SHA-256 `5d9b1837eb58b0760463e99021a53fe6e82d5cd2457141945445ed6df06ff3f7`
and 657,272,400 bytes. The metadata acquisition receipt is pinned by SHA-256
`3a2b156289b7f7f11d20ef5026ffe9f19893333d155f0acb6e326f027ceb0747`.
Of the required 47 H5 files (NP3 calibration + 46 named NP5 poses), 38 are
already hash-sealed extracted files. A read-only tar inventory confirms the
remaining nine NP5 files (angles 0, 300, 306, 312, 318, 330, 336, 342,
348), each exactly 2,296 bytes. The run would stage only these nine under its
own output root; no network, full archive extraction, or source overwrite.

Default `python3 -m scripts.classical_backend.openmvg_mustard_gt_posthoc`
is read-only: it hashes both exports, the source model, the metadata receipt
and cached archive, checks all extracted metadata hashes, inventories the nine
archive headers, reserves disk and opens **no GT H5 datasets**. `--run` is
separately gated for review and writes only to fresh
`/Volumes/backups/code/crispmesh-data/openmvg-mustard-gt-posthoc-001`.
The projected and live gates retain 11 GiB free on external and internal
disks, allow at most 2 MiB total output and 64 KiB staged metadata, and cap
wall time at 300 s (H5 reads have individual 10 s subprocess timeouts).
Staged/source hashes and exact inventories are checked again after scoring;
the receipt and report preserve input pins, staged H5 hashes, status and
failure artifacts.

## Interpretation, not an acceptance claim

For each arm independently, the existing
`checkerboard_pose_reference_eval.compare` fits one proper positive-scale
center Sim(3) and reports center residuals, center residuals normalized by
reference median orbit radius, leave-one-out normalized center residuals,
orientation residuals, covariance singular values and per-frame values. A
paired per-frame table records `NONE − ADJUST_ALL` for normalized center,
normalized leave-one-out and orientation errors. OpenMVG source-center units
are arbitrary; target-center units are Berkeley H5 pose-translation units,
not board-square units or independently established meters. Legacy shared
evaluator fit-field names are relabeled accordingly.
The exact shared Sim(3)/leave-one-out implementation is pinned by source
SHA-256 `f24442884ecb9e102ee5ad96fb4727b56dc30e94d0f92aa783c6350704878c7b`;
the preflight also records the installed NumPy version and requires `h5dump`.

Reference pose convention follows the existing checkerboard comparison:
`H_NP3_from_table = H_NP3_from_NP5 @ inverse(H_table_from_reference_camera)`,
with world-to-camera OpenCV axes. Orientation agreement is conditional on
these matrix-name/axis conventions; the board-frame offset and depth have
not independently validated them. A near-planar orbit weakens the third
alignment axis, so singular values and leave-one-out errors matter. Both
SfM arms remain 46/48 incomplete regardless of posthoc scores. No metric
implies mesh accuracy, absolute camera truth, or GPL/AGPL shipping clearance.
