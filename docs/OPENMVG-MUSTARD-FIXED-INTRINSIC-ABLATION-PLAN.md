# Mustard fixed-intrinsic OpenMVG SfM ablation — review gate

This is a separate, one-shot, evaluation-only **SfM-stage** trial. The existing
mustard `openmvg-mustard-photo-sfm-001` result remains intact and failed its
48/48 sparse-registration gate (46 poses, 222 tracks, 1497 residuals). The
trial asks whether holding the **photo-derived** initial intrinsic fixed changes
that outcome; it does not assume an improvement or certify geometry.

## Frozen inputs and sole model change

The worker pins the -001 receipt SHA-256
`e45c400dcf71a3ba86fa22f7af2138418d6dc511fddcd6d8470ef33599804744`
and checks its complete 170-file output inventory byte-for-byte before and
after the run. It copies all 105 flat files in sealed `-001/matches` (including
the listing JSON, `matches.e.bin`, `image_describer.json`, all 48 `.feat` and
`.desc` pairs, and auxiliary files) into a fresh `-002/matches` directory,
then hashes every staged file. The listing JSON stays byte-identical, with
`root_path` pointing at the 48 sealed -001 TRAIN JPEGs; those JPEGs are
individually rehashed before and after. The native process receives only the
staged match directory and new sparse-output directory as writable paths.

The JSON contains one `pinhole_radial_k1` intrinsic: 1280×1024 image,
focal 1536 px (= 1.2×image width), principal point [640, 512], k1=0, and
no input poses/structure/control points. No Berkeley calibration/poses/mesh,
board corners, masks, scanner or held-out photos enter this worker. It reuses
the exact sealed `openMVG_main_SfM` binary SHA-256
`3d159212e82f16036b797337d3369ce6dfceabe17ff1745c48aaa2b545ab23f2`
and verifies the four-/fifth-target receipts. The source parser and compiled
no-argument usage both expose `-f NONE`; in pinned source this holds existing
intrinsic parameters constant, while the default extrinsic refinement remains
`ADJUST_ALL`. Relative to -001's SfM command, input paths point to staged
byte-identical files, output points to fresh `-002/sparse`, and the sole model
option change is `-f ADJUST_ALL` → `-f NONE`. No earlier stage is rerun.

## One-shot boundary and stop conditions

Default `python3 -m scripts.classical_backend.openmvg_mustard_fixed_intrinsic_ablation`
is a read-only preflight. The explicit `--run` entry point is **not approved by
this plan**; review the preflight/command first. It may write only to fresh
`/Volumes/backups/code/crispmesh-data/openmvg-mustard-photo-sfm-fixed-002`.
It never cleans or reuses -001 or an existing -002. External free space must
exceed an 11 GiB floor plus the full 256 MiB output reservation; internal
space must exceed 11 GiB. The SfM process is capped at 1 GiB observed
process-tree RSS, 300 s wall, 600 s CPU, 4 MiB log and 256 MiB total output;
thread environment is capped at two. Stage receipt/log, exit code, model hash,
HTML report counts and non-self-referential output inventory are preserved on
success or failure. Exceeding a cap stops the process group and retains the
partial output for audit.

## Comparison after a separately approved run

Compare -002's SfM report with the sealed -001 `ADJUST_ALL` report on the same
48 TRAIN names: registered poses (primary sparse gate), tracks and residuals.
Require 48/48 poses and nonzero tracks/residuals only to reach *pending geometry
review*; this is not 3D acceptance. If a new model exists, a separately reviewed
diagnostic export can inspect its final intrinsic and named camera poses, then
apply the existing orbit/geometry checks. Do not infer pose quality from counts
or a nonempty `sfm_data.bin`. Keep any Berkeley/reference-based scoring in a
separate, subsequently authorized lane. This is an evaluation oracle only;
there is no GPL/AGPL shipping or App Store clearance claim.
