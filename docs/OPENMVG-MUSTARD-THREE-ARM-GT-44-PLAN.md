# Mustard three-arm camera comparison on common 44 — review gate

This is an **evaluation-only posthoc** comparison of sealed OpenMVG mustard
`ADJUST_ALL` incremental, fixed-intrinsic `NONE` incremental, and `GLOBAL`
`ADJUST_ALL` camera-pose exports. No Berkeley pose or calibration was an input
to any SfM arm; this worker has no reconstruction command. The two
incremental exports each lack `NP3_042.jpg` and `NP3_048.jpg`; the GLOBAL
export lacks `NP3_252.jpg` and `NP3_276.jpg`. The **common population is
exactly 44 named TRAIN views**, excluding all four. Each arm gets its own
fresh proper positive-scale Sim(3) fit on these same 44; the prior two-arm
46-view fit errors are not reused or compared as if they were 44-view scores.

The ADJUST_ALL and NONE export receipt/JSON/model seals are rechecked through
the prior two-arm validator. GLOBAL export receipt SHA-256 is
`ac20487ca1932e7e42046c43b5fbae768383051f53a61729f4bb5d49dac7bebd`,
JSON SHA-256 is
`6382b68e07815d730f5b428e23004bac99a210fced8b5c9a5706e8503a2da709`,
and source model SHA-256 is
`eb5d96ce4379ee32bdef3f946f833fd2c7a85b0a5c818faa4ce799766f74fc00`.
The GLOBAL source receipt, log, full inventory, 105 staged match files and
original photo-derived source are checked through its sealed export gate.
All three JSONs are parsed with the cereal `id_pose` → extrinsic-key mapping,
not array order, before the 44-name intersection is frozen.

For Berkeley metadata, this run reuses **only existing tiny H5 files**: the
original hash-sealed extracted NP3 calibration/NP5 poses and nine NP5 files
staged by the previous two-arm result. It pins that result receipt SHA-256
`863472bc0af7e69d0b16602dd8ea05ed59ead1a106d076b1f8ad01f002a1643e`
and report SHA-256
`e6a791a8e16deba1ac6856b8f4eb90df7ce6ec4e2a092a53eb4b955892e8d035`,
checks its staged artifact inventory and original metadata receipt, and
rehashes the **45** H5 files needed for 44 cameras plus calibration. The
large cached archive is not opened or re-extracted in this trial. The prior
report is read only to verify its seal/schema; no earlier score value is
used in the new fits. The exact shared
`checkerboard_pose_reference_eval.compare` source SHA-256
`f24442884ecb9e102ee5ad96fb4727b56dc30e94d0f92aa783c6350704878c7b`
is checked before and after scoring.

Default `python3 -m scripts.classical_backend.openmvg_mustard_three_arm_gt_posthoc`
is read-only and opens **no GT H5 datasets**; a NumPy-capable reviewed Python
runtime and `h5dump` must be present for preflight. Scoring `--run` is
separately gated and may write only receipt/report under fresh external
`/Volumes/backups/code/crispmesh-data/openmvg-mustard-gt-posthoc-three-arm-001`.
It retains at least 11 GiB free externally **plus** a full 1 MiB output
reservation and at least 11 GiB internally, checks disk during each H5 read,
caps total wall time at 300 s, and preserves partial receipt/report on
failure. Sources, algorithms and H5 hashes are rechecked after scoring.

Per arm, the shared evaluator reports Sim(3) conditioning, center residuals
in Berkeley pose-translation units and normalized by median reference
camera radius, leave-one-out normalized center residuals, orientation errors
and per-frame values. Paired per-frame differences are `NONE−ADJUST_ALL`,
`GLOBAL−ADJUST_ALL`, and `GLOBAL−NONE` on the identical 44 names. OpenMVG
source-center units are arbitrary, **not board squares**; legacy fit-field
labels are renamed. The target unit is the supplied Berkeley pose-translation
unit, not independently established meters. Orientation comparisons remain
conditional on NP3/NP5 matrix-name and OpenCV axis conventions; neither the
board-frame offset nor depth independently validates them. A near-planar
orbit weakens third-axis alignment, so covariance singular values and
leave-one-out errors matter. All source models remain 46/48 incomplete;
these scores do not prove absolute camera or mesh accuracy and grant no
GPL/AGPL shipping or App Store clearance.
