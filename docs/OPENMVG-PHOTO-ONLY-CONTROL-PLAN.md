# OpenMVG v2.1 photo-only SfM control, M1

Status: **read-only preflight passed; no OpenMVG photo run authorized**.
This is an evaluation oracle. The built fork and its linked components have
unresolved license closure; no shipping, permissive, GPL-free, or App Store
claim follows from a successful run. The fork source pin is OpenMVG v2.1
`01193a245ee3c36458e650b1cf4402caad8983ef` with only the reviewed LiGT
OFF CMake exclusion. The build tree stays untouched by the photo control.

## Dependency gate discovered in the pinned source

The initial four built targets were insufficient for a standard SfM sequence.
`main_ComputeMatches.cpp` accepts `-o` as a **filename** and writes putative
matches only; it has no `-g` switch. The older Sphinx `ComputeMatches.rst` and
the earlier mustard M1 plan describe the pre-split `-o DIR -g e` interface and
must not be used for this pin. `main_GeometricFilter.cpp` is a separate fifth
target, takes `-m PUTATIVE -o GEOMETRIC -g e`, and performs essential-matrix
verification. `main_SfM.cpp` then reads the verified file with `-M
matches.e.bin`. Feeding putative matches directly to SfM would change the
control. The fifth target has since passed a separate two-step Ninja closure
and one-shot bounded build, with no new static or dynamic libraries. The
[photo supervisor](../scripts/classical_backend/openmvg_photo_control.py)
pins the four-target license receipt SHA-256
`9aec9a7ac01c76f683416cd8eb47c57bbcbfa006a642348568a04298570078c1`,
the fifth `geometric-filter-build-manifest.json` SHA-256
`4b3dc5ac03ea2232067bebabc0b4d41dc5bfb91076bf0e31211162f32842774b`,
the fifth `geometric-filter-license-receipt.json` SHA-256
`49db5dca401c827cac53a5b1337256effd198ed36de23de283b2b32548e53dc2`,
and the fifth binary SHA-256
`49a5ca029356f3f8bb3eb68eaae58d98f77204b2b4cdf85f52aa0448f0ab1b77`.
It checks the four original binary hashes against their sealed receipt.
The fifth receipt remains explicitly evaluation-only; no install or network
fetch is part of this control.

## Sceaux first: fixed input and process contract

Use exactly the 11 JPEGs in
`.local-tools/upstream-control/openmvs-sceaux-v23/images`, copied byte-for-byte
to fresh external output
`/Volumes/backups/code/crispmesh-data/openmvg-sceaux-photo-sfm-001`.
The upstream sample manifest SHA-256 is
`1ee34cff7e6d75fca9f56de4aa5644b45a382030af54a474824ada6af77316a3`;
it lists the 11 exact photo SHA-256 values and byte sizes. The supervisor
requires that seal, exact image inventory and all individual hashes both
before and after copying. The sample `scene.mvs`, supplied upstream cameras,
dense scene, meshes, logs and README are never copied into the worker tree or
passed to any CLI. The sample photo rights remain those stated in its
manifest; outputs are local evaluation artifacts.

The Sceaux command contract is:

```text
SfMInit_ImageListing -i IMAGES -o MATCHES -f 3398.4 -c 2 -g 1
ComputeFeatures -i MATCHES/sfm_data.json -o MATCHES -m SIFT_ANATOMY -p NORMAL -n 2
ComputeMatches -i MATCHES/sfm_data.json -o MATCHES/matches.putative.bin -r 0.8
GeometricFilter -i MATCHES/sfm_data.json -m MATCHES/matches.putative.bin -o MATCHES/matches.e.bin -g e
SfM -i MATCHES/sfm_data.json -m MATCHES -M matches.e.bin -o SPARSE -s INCREMENTAL -f ADJUST_ALL
```

Each displayed command means the exact `openMVG_main_` executable from the
sealed fork. All 11 JPEGs are 2832×2128; focal 3398.4 px is the disclosed
1.2×width photo-only heuristic, with radial-one camera model and grouped
intrinsics. No pose prior, initial camera pair, reference calibration or
external match database is provided. Intrinsics may refine from the heuristic.
The default automatic pair initializer remains an algorithm decision to
report, not a supplied pose. Feature extraction is CPU only, with two feature
threads and process environment thread caps of two.

Read-only preflight is the default command:

```text
python -m scripts.classical_backend.openmvg_photo_control
```

The read-only preflight now passes with all five targets. It hashes the sealed
photos and executable files and reads receipts; it does not launch OpenMVG.
Only after a review of all five target hashes, command syntax and this plan
may the one-shot
`--run-sceaux` path be used. It refuses an existing output directory and
never cleans failed output. The external output allocation is at most 1 GiB;
before starting it reserves that allocation plus a 1 GiB margin above the
10 GiB floor. Both internal and external devices remain at least 11 GiB free
during work. Each process-tree RSS is capped at 4 GiB, each stage log at
16 MiB, total wall time at 1800 seconds and stage times at 120/420/420/420/420
seconds. Temporary files point into the new external output. A violation
terminates only the launched process group and preserves its receipt.
Each child also has an OS CPU-time limit of twice its stage wall-time cap,
matching the two-thread allowance.

Receipts contain exact input and binary hashes, stage command and status,
wall time, peak sampled RSS, log hash/size, required artifact hash, output
bytes and final file inventory (excluding the receipt itself). The pinned
`sfm_report.cpp` emits `#views`, `#poses`, `#intrinsics`, `#tracks`, and
`#residuals` in `SfMReconstruction_Report.html`; the supervisor requires
unique, consistent counts. Eleven poses plus nonzero tracks and residuals
yield **completed, pending geometry review**. Fewer poses or zero sparse
support are recorded as a failed quality gate even if the native CLI exits
zero. Post-run neutral review must inspect the binary model for finite
intrinsics, poses and points, reciprocal observations, reprojection
distribution and visual coverage. Compare cameras with the upstream scene
only after freezing and
hashing the photo-derived model; that comparison is evaluation-only software
agreement, not physical ground truth. No dense reconstruction is included.

## Mustard 48: separate subsequent decision

Only after the Sceaux run and review, prepare a **separate** fresh external
output and one-shot supervisor mode for the exact 48 TRAIN JPEGs in
`/Volumes/backups/code/crispmesh-data/mustard-sfm-train-001/images`.
The sealed `train-names.txt` hash is
`a81d1647109db27991c39c5fe1a9ae308c04081f8ada49d8b0f5ab1c15245544`;
`stage-report.json` is
`bbcba624f6a51f82e0ebdebf6e7e4cda10804bc8df34cfd0d5b3242983a225d0`.
That report binds every TRAIN JPEG and reviewed coarse mask. The mustard
experiment must remain photo-only: do not feed board corners, Berkeley camera
poses, NP3 calibration, reference mesh, scanner depth or 12 held-out photos.
If masks are used, stage exactly the reviewed coarse masks as
`NP3_006_mask.png` style names after checking their binary values, dimensions
and photo correspondence; disclose that masks may include support. The
previous [mustard plan](OPENMVG-MUSTARD-M1-PLAN.md) defines a candidate
fixed-1536 px starting focal and orbit-fold rejection gates, but its matching
command must be replaced by the verified five-stage interface above before
review. Mustard needs its own resource allocation, photo hashes, frozen
commands and approval; Sceaux success does not authorize it.
