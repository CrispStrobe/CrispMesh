# OpenMVG mustard 48 TRAIN photo-only SfM control

Status: **read-only preflight and synthetic review only; no mustard photo run**.
The separate Sceaux `-002` run completed its five-stage, 11/11 camera control.
That establishes the local OpenMVG executable path, not a rotating-object
quality result. Mustard uses a new one-shot output at
`/Volumes/backups/code/crispmesh-data/openmvg-mustard-photo-sfm-001`; it does
not reuse or modify either Sceaux output. OpenMVG remains an evaluation-only
oracle without a shipping-license conclusion.

## Sealed input and camera assumption

Use exactly the 48 `NP3_*.jpg` TRAIN photographs in
`/Volumes/backups/code/crispmesh-data/mustard-sfm-train-001/images`.
`train-names.txt` SHA-256 is
`a81d1647109db27991c39c5fe1a9ae308c04081f8ada49d8b0f5ab1c15245544`;
`stage-report.json` SHA-256 is
`bbcba624f6a51f82e0ebdebf6e7e4cda10804bc8df34cfd0d5b3242983a225d0`.
The report binds every TRAIN JPEG hash. Read-only preflight checks the image
header of each sealed JPEG with macOS `sips` and requires 1280×1024 for all
48 photos. The initial focal length is fixed in advance at
`1.2 × 1280 = 1536` pixels, a disclosed image-width heuristic. It is not
the Berkeley NP3 calibration. SfM may refine intrinsics (`ADJUST_ALL`), as
the Sceaux control did.

The supervisor reads the sealed names/report and JPEGs during preflight, then
copies and re-hashes only those 48 JPEGs. The stage report contains mask
metadata, but no mask file is read, staged or passed to a CLI. No board
corners, rig calibration, Berkeley per-angle poses,
scanner data, reference mesh, depth, 12 held-out photos, prior-camera seed,
or external matches enter the worker. The output image directory contains
JPEGs only. TRAIN angle labels are ordering metadata, not pose observations.

## Frozen five-stage command contract

The same five pinned v2.1 LiGT-off executables and sealed build receipts from
the [Sceaux plan](OPENMVG-SCEAUX-PHOTO-CONTROL-002-PLAN.md) are required.
Preflight calls each binary without arguments to compare every planned short
flag against its compiled usage. This is the OpenMP-off interface; feature
`-n` is absent. The exact photo-only commands are:

```text
SfMInit_ImageListing -i IMAGES -o MATCHES -f 1536 -c 2 -g 1
ComputeFeatures -i MATCHES/sfm_data.json -o MATCHES -m SIFT_ANATOMY -p NORMAL
ComputeMatches -i MATCHES/sfm_data.json -o MATCHES/matches.putative.bin -r 0.8
GeometricFilter -i MATCHES/sfm_data.json -m MATCHES/matches.putative.bin -o MATCHES/matches.e.bin -g e
SfM -i MATCHES/sfm_data.json -m MATCHES -M matches.e.bin -o SPARSE -s INCREMENTAL -f ADJUST_ALL
```

The `openMVG_main_` prefix and sealed full binary path apply to each displayed
name. Putative matching, geometric filtering and SfM are separate steps.
SfM chooses its own seed pair; no pose or chosen pair is supplied.

## One-shot limits, receipts and review gate

The external output path must not exist before preflight. The output tree is
limited to 1 GiB. Before starting, the external device must have the full
1 GiB allocation plus an 11 GiB reserve; during execution, both devices
must remain above 11 GiB free (10 GiB floor plus 1 GiB margin). Each child
has a 4 GiB process-tree RSS cap, 16 MiB stage log cap, two-thread math
environment and OS CPU-time cap. The inherited total wall cap remains
1800 seconds even though the individual stage caps sum to more; stage caps
are 120/600/900/600/900 seconds. A failure terminates only the launched
process group and retains all partial files and receipts. No retries, installs,
network access, dense reconstruction or cleanup are included.

Read-only preflight:

```text
python -m scripts.classical_backend.openmvg_mustard_photo_control
```

The `--run-mustard` path requires a separate review. Per-stage receipts record
commands, elapsed time, sampled peak RSS, log and artifact hashes, status,
and output bytes. A final inventory excludes the self-referential receipt.
The pinned SfM HTML report must show exactly 48 listed views and 48 poses,
plus nonzero tracks and residual observations, to reach **pending geometry
review**. Partial registration is a failed sparse gate. Even 48/48 poses can
hide a folded orbit: subsequent neutral review must inspect finite
intrinsics/poses/points, reciprocal tracks, reprojection distribution, full
turn coverage, adjacent camera steps and cyclic opposing camera pairs. The
predeclared fold thresholds remain in the earlier
[mustard M1 plan](OPENMVG-MUSTARD-M1-PLAN.md). No reference or held-out data
may tune this run. Any comparison to independent scanner/reference geometry
must occur only after the photo-derived model and analysis protocol are sealed.
