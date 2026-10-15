# OpenMVG HIGH bunny sparse camera audit

This is a diagnostic review of the frozen image-only HIGH sparse run at
`/Volumes/backups/code/crispmesh-data/openmvg-bunny-high-photo-sfm-001`.
No scanner geometry, supplied PRO poses, calibration, or mask was used by the
SfM producer. Its receipt reports 73/73 poses, one intrinsic, 6,526 tracks,
and 23,619 residuals. Those counts alone do not establish camera or mesh
quality. The source receipt and `sparse/sfm_data.bin` hashes were checked
before and after the diagnostic export.

The existing sealed OpenMVG v2.1 `openMVG_main_ConvertSfM_DataFormat` binary
exported **poses and intrinsics only** (`-V -I -E`) to fresh
`/Volumes/backups/code/crispmesh-data/openmvg-bunny-high-pose-audit-001/sfm_camera_poses.json`.
The invocation was capped at 120 seconds and 2 MiB output, with at least
11 GiB free on both volumes. It finished with 102,755 bytes: 73 views,
73 extrinsics, one intrinsic, and no structure or control points. The
exporter did not alter the sparse source. This JSON is a diagnostic copy,
not a modified camera model. A separately authorized `-V -I -E -S` export
to fresh `openmvg-bunny-high-structure-audit-001/sfm_structure.json` was
bounded to 16 MiB and 120 seconds under the same disk floor. It contains
all 6,526 tracks and 23,619 saved 2D observations; the source hashes were
again unchanged afterward.

## Acquisition order and trajectory

The prepared `frame_####.png` names follow lexical source filename order;
the original numeric `bunny_N_rgb.png` order is recovered from the sealed
`build-opencv/bunny-gamma05-clahe2/prepare-manifest.json`. The unchanged
`scripts/object_motion/orbit_plausibility.py` gate requires an even number of
slots, so this check uses numeric `bunny_0` through `bunny_71` (72/72 posed)
and leaves the also-registered `bunny_72` out of the gate. No supplied camera
poses or scanner points enter that calculation.

The necessary orbit check **passes**: 360° winding, zero reversed significant
steps, all optical axes inward, no collapsed opposing pairs, and no criterion
failure. The largest adjacent center step is 0.179883 of median orbit radius;
the largest adjacent orientation step is 10.18485°. The center-plane PCA
minor/major ratio is 0.96467. Passing this broad check does not validate
every local baseline.

One neighboring pair deserves explicit treatment. Numeric `bunny_13`
(`frame_0004`) to `bunny_14` (`frame_0005`) has a 0.000738 model-unit center
step, only 0.24% of the 0.302818 median neighboring step, and a 0.01158°
rotation versus the 5.04496° median. The prepared RGB images are nearly
identical: grayscale mean absolute difference 0.629/255 and quarter-scale
pixel correlation 0.999626. Adjacent `12→13` and `14→15` differences are
3.277 and 3.708/255, with correlations 0.977866 and 0.973133. Both photos
and their prepared outputs have distinct SHA-256 hashes. Thus the tiny
recovered baseline is consistent with the captured RGB pair being nearly
stationary; it is not evidence by itself of a camera-estimation defect.

As a separately labeled post hoc context, the supplied depth-only,
locked-direction PRO `poses_metric.json` reports a 4.86134° `13→14` rotation
and 4.8613–5.0095° rotation for all 72 numeric neighboring pairs. Its
`T_CO` matrices yield essentially constant `-Rᵀt` centers under a simple
world-to-camera interpretation. That convention and the depth pipeline's
prior are not independently validated here, so the PRO file is **not camera
ground truth** and the angular disagreement is not an SfM error score.

## Intrinsics and object region

The exported model has one shared `pinhole_radial_k1` camera for 1749×1155
images: focal length 2384.323 px, principal point (963.372, 568.866) px,
and k1 = −0.408783. These are finite and shared; they are not a calibrated
camera validation. The separately supplied `rgb_optic.json` is a 583×385
Brown5 RGB optic calibration and was not an input. Its distortion model
and coordinate convention differ, so coefficient equality is not expected.

The existing `sparse/cloud_and_poses.ply` has 73 green camera vertices and
6,526 white sparse points. The green positions exactly match the exported
centers. Every **saved** 2D track observation was joined to its own image
and tested at its recorded pixel coordinate against the hash-verified,
photo-only coarse bunny mask. All 23,619 observations are in bounds;
22,695 (96.09%) lie inside. Per-view support ranges from 92.41% to 98.99%
(median 96.11%); 6,154 tracks have at least two inside observations.
This is stronger object-region evidence than projecting all points into
every image. The masks still include some contact shadow and truncate some
lower-body pixels, so the count is neither exact silhouette support nor
mesh shape accuracy. The native SfM report's low residual summaries are
internal fit statistics, not independent geometry truth.

## Decision and sealed inputs

The HIGH sparse model passes a necessary global orbit check and has strong
coarse object-region support from actual saved observations. The separate
hash-bound [camera review receipt](../tests/evidence/bunny-openmvg-high-camera-review-001.json)
approves **exploratory OpenMVS continuation only**. It does not accept camera
accuracy, physical scale, sparse 3D shape, or any future dense mesh. The
near-duplicate pair remains a local-baseline limitation. Dense processing
is a composed continuation, and its mesh needs separate visual and surface
evaluation. No historical surface score is changed here.

| Evidence | SHA-256 |
| --- | --- |
| HIGH source `receipt.json` | `97cf72ab72c75a88f04d97baa29c6de49a1640103d7143dd151bc2efe54c507d` |
| HIGH source `sparse/sfm_data.bin` | `aea2b3129cb317663435461fd12fa4c0078c57211fc1c08cf7b1a29ad6902975` |
| Pose-only diagnostic JSON | `8552a165dac3181e66b57ef1c585af0ab47831c944b0983ff40fe04373a8960f` |
| Saved-structure diagnostic JSON | `3930629a8da1baca576984d9554d226f7f54f1b6e9f86be7e76227290d59a1b9` |
| Sealed converter binary | `95bebb65afd1374aadae21aecc5f4cf66f4257432477d54ceae22c4cc35636b0` |
| Prepared-photo manifest | `eca0bfa60badd7fa5c9b5311e7f413a3f337159c158f856066086e44b0a90ecd` |
| Sparse cloud and cameras PLY | `e1800fa60285169b79c0ff3a6d523b6086668f72b64b42cb4ee6daabd57b998b` |
| Photo-only mask manifest | `e5b242f837ee0971408b976ee9fed45e8ee3c660e56fda3f0db2e7a6749c6521` |
| Supplied PRO pose metadata (post hoc) | `446a3124234704ba649d8b16ed9938835d9e21d16764b9768e72c3a99e417ef7` |
