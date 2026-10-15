# Mustard OpenMVG GLOBAL SfM control — review gate

This is a separate, one-shot, **photo-only** engine control on the sealed 48
mustard TRAIN photographs, not a repair of the failed incremental model or a
Berkeley-guided reconstruction. The existing -001 incremental `ADJUST_ALL`
result and fixed-intrinsic -002 result remain untouched. There is no native
SfM run authorized by this plan.

## Pinned source/CLI contract

The exact existing OpenMVG v2.1 `openMVG_main_SfM` binary is pinned by SHA-256
`3d159212e82f16036b797337d3369ce6dfceabe17ff1745c48aaa2b545ab23f2`;
the four-/fifth-target build receipts are checked as in the earlier controls.
The reviewed `main_SfM.cpp`, GLOBAL engine implementation and translation
averaging header are independently SHA-256-pinned in the preflight; the
compiled binary seal remains the executable authority.
Pinned `main_SfM.cpp` parses `-s GLOBAL`, instantiates
`GlobalSfMReconstructionEngine_RelativeMotions`, and applies the same
`-f ADJUST_ALL` intrinsic refinement policy after engine creation. Its `-M`
file lookup reads the explicit staged `matches.e.bin`. The compiled binary's
no-argument usage is inspected read-only for every planned option before a
run. Relative to the -001 incremental SfM command, after substituting fresh
staged input/output paths, **only `-s INCREMENTAL` → `-s GLOBAL` changes**.
The source defaults remain rotation averaging L2 (`-R 2`) and translation
averaging SoftL1 (`-T 3`); neither flag is passed, and LiGT (`-T 4`) is not
requested or compiled. The source's GLOBAL `Adjust()` has initial
intrinsic-fixed BA passes followed by an intrinsic-refining pass when
`ADJUST_ALL` is selected; thus the requested policy is supported, but the
optimization trajectory is not identical to incremental SfM.

The GLOBAL engine first retains the largest bi-edge-connected view graph,
estimates relative rotations, globally averages rotations/translations, then
builds structure from validated triplet tracks. A graph without sufficient
bi-edge connectivity/triplets may fail even if incremental SfM produced a
46/48 model. Conversely, GLOBAL does not inherently fix repeated-texture,
turntable aliasing or weak orbit geometry. This is an algorithmic control,
not a promised improvement.

## Sealed input and bounded one-shot output

The input is the exact -001 photo-derived listing scene: one 1536 px
(1.2×1280) image-width heuristic intrinsic, no input extrinsics/structure or
control points. Its `root_path` names only the sealed 48 TRAIN JPEGs. The
worker validates -001 receipt SHA-256
`e45c400dcf71a3ba86fa22f7af2138418d6dc511fddcd6d8470ef33599804744`,
the complete -001 output inventory and all 48 JPEG hashes, then stages **all
105** byte-identical flat match-directory files into fresh output/matches,
including the scene, region descriptors/features and essential matches. It
hashes staged files and rechecks staged/source inventories after SfM. No
Berkeley K/poses/mesh, board corners, masks, scanner, depth or held-out photo
enters the command. No listing, feature, matching or geometric-filter stage
is rerun; no new binary or library dependency is built.

Default `python3 -m scripts.classical_backend.openmvg_mustard_global_sfm_control`
is a read-only preflight. Explicit `--run` requires separate review and may
write only to fresh external
`/Volumes/backups/code/crispmesh-data/openmvg-mustard-photo-global-sfm-001`.
The prior output is never cleaned or overwritten. The same resource envelope
as the fixed-intrinsic SfM-only control applies: at least 11 GiB free on the
internal disk, 11 GiB free **plus** a full 256 MiB output reservation on the
external disk, 1 GiB observed process-tree RSS, 300 s wall, 600 s CPU, 4 MiB
log and 256 MiB total output; two-thread environment. Failure stops the
process group and preserves the stage log, receipt and partial output.

After any separately approved run, compare registration count, tracks and
residuals to the sealed incremental `ADJUST_ALL` 46/48 result on the same 48
names. A 48/48, nonzero sparse report would only advance to *pending geometry
review*. A model file or favorable count alone is not a geometric quality
claim. Any JSON export or posthoc GT comparison needs its own sealed review
gate; reference data must never feed this SfM control. Evaluation only: no
GPL/AGPL shipping or App Store clearance claim.
