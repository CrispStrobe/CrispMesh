# OpenMVG mustard TRAIN: frozen intrinsic-`NONE` ablation contract

**Frozen pre-run plan; the completed result is recorded separately in
[the `-002` result](OPENMVG-MUSTARD-INTRINSIC-NONE-002-RESULT.md).**
The comparison asks whether holding the
initial camera intrinsics constant changes the failed registration/sparse
outcome of the sealed [photo-only `-001` run](OPENMVG-MUSTARD-PHOTO-001-RESULT.md).
It does not test feature extraction, matching, masks, dense reconstruction,
physical pose accuracy, or a shippable backend.

## Pinned semantics and input seal

The evaluation binary is `openMVG_main_SfM` SHA-256
`3d159212e82f16036b797337d3369ce6dfceabe17ff1745c48aaa2b545ab23f2`,
built from OpenMVG v2.1 source commit
`01193a245ee3c36458e650b1cf4402caad8983ef` with the reviewed LiGT-off
oracle patch. The baseline `-001` receipt SHA-256 is
`e45c400dcf71a3ba86fa22f7af2138418d6dc511fddcd6d8470ef33599804744`.
Its SfM stage used `-s INCREMENTAL -f ADJUST_ALL` with the input paths below.

The pinned [`main_SfM.cpp`](https://github.com/openMVG/openMVG/blob/01193a245ee3c36458e650b1cf4402caad8983ef/src/software/SfM/main_SfM.cpp)
documents `ADJUST_ALL` as the default and `NONE` as holding intrinsic
parameters constant. The pinned
[`Cameras_Common_command_line_helper.hpp`](https://github.com/openMVG/openMVG/blob/01193a245ee3c36458e650b1cf4402caad8983ef/src/openMVG/cameras/Cameras_Common_command_line_helper.hpp)
parses `NONE` into a nonzero enum value; the
[`BA_Ceres` implementation](https://github.com/openMVG/openMVG/blob/01193a245ee3c36458e650b1cf4402caad8983ef/src/openMVG/sfm/sfm_data_BA_ceres.cpp)
holds that intrinsic parameter block constant and does not update it from
the optimized block. The incremental engine still permits extrinsic and
structure adjustment. Other initializers or non-BA computation could cause
nonlinear downstream differences; this is an **intrinsic-refinement-policy**
ablation, not a guarantee that only the final focal-length number differs.

Copy these existing `-001` files byte-for-byte into a **fresh** ablation
`matches/` directory, then point SfM at the copies; do not regenerate or
rewrite the originals:

| Input | SHA-256 |
| --- | --- |
| `matches/sfm_data.json` | `8a61622cc22f76a6bb7dbcbf5a4e00725afd6a57b1e20095dabafab931112033` |
| `matches/image_describer.json` | `e1edff9105c9fe77d965d17bc8ef9a4bf107495fb17eef7fbbf13152e5afcab3` |
| `matches/matches.e.bin` | `1e2eebfee9a5326431f4d9c415d88c957e8e30b99899302e33f0397daca2becc` |

The match directory contains exactly **105 receipt-inventoried files**.
Its **48 `.feat` and 48 `.desc` files** match their individual receipt hashes;
the sorted UTF-8 lines `relative-path<TAB>sha256<LF>` over those 96 files
hash to `557519b02feebae766fd6a9d453637b57d3e79e46f6dacb40d98822405d10322`.
The same digest over all 105 files is
`9fc1d33ccccca4b4ab47e5c3f6dffc3f116a8a07e02cf952a92e11c0b7e5aa2e`.
All files matched the sealed receipt during this read-only audit. The SfM
loader reads `.feat` positions, `image_describer.json`, and the specified
`matches.e.bin`; `.desc` and putative-match files remain frozen upstream
artifacts but are not read by this SfM stage. `sfm_data.json` loads views and
initial intrinsics, **not** baseline reconstructed poses.

## One-variable comparison

Hold fixed the binary, initial `sfm_data.json`, 48 TRAIN images addressed by
it, all consumed match/feature bytes, `-s INCREMENTAL`, `-M matches.e.bin`,
all implicit/default SfM flags and the two-thread setting. Stage
all **105** receipt-inventoried match-directory files (including the initial
scene, describer, essential-matrix matches, and 96 region files) into fresh
`/Volumes/backups/code/crispmesh-data/openmvg-mustard-photo-sfm-fixed-002/matches`;
verify each staged SHA-256 against the `-001` receipt before launch. Change
only `-f ADJUST_ALL` to `-f NONE` in the SfM algorithm; `-i`, `-m`, and `-o`
change paths solely to isolate the byte-identical inputs and fresh outputs.
No Berkeley/reference poses, held-out images, masks, scanner
assets, dense/MVS, or threshold search may enter the ablation.

The planned argument sequence is:

```text
openMVG_main_SfM
  -i /Volumes/backups/code/crispmesh-data/openmvg-mustard-photo-sfm-fixed-002/matches/sfm_data.json
  -m /Volumes/backups/code/crispmesh-data/openmvg-mustard-photo-sfm-fixed-002/matches
  -M matches.e.bin
  -o /Volumes/backups/code/crispmesh-data/openmvg-mustard-photo-sfm-fixed-002/sparse
  -s INCREMENTAL
  -f NONE
```

Source-level path review: `main_SfM.cpp` loads `sfm_data.json`,
`image_describer.json`, features and matches; the providers' `load` methods
read files into memory. The copied scene's `root_path` intentionally remains
the sealed `-001/images` directory. `Features_Provider::load` combines it with
view paths to derive image **basenames** for the staged `.feat` files; the
inspected SfM main, sequential engine and providers show no JPEG pixel load.
That original 48-photo directory is still an absolute path embedded in the
input scene and must remain sealed/available. The main and incremental engine
direct reports, intermediate PLYs, `sfm_data.bin`, and `cloud_and_poses.ply`
to fresh `-o`, not to `-m`. This is source-level evidence, not a
filesystem-enforced guarantee: independently hash the entire original
105-file match directory, all 48 original photos, receipt, and all staged
files **before and after** and fail on any difference. Require a nonexistent
output root before starting; never use `-001/sparse` as an input or output.

For a separately approved run, the draft one-shot supervisor tightens the
evaluation caps: no network/install/rebuild, at most two CPU threads,
1 GiB peak RSS, 300 s SfM wall time, 600 s CPU time, 4 MiB log,
256 MiB new output, and at least 11 GiB free
on both internal and external disks before launch and throughout. Seal the
command, binary, input hashes, output inventory, log, wall/RSS, and postflight
free space in a fresh receipt. At this audit, external free is 13,973,180 KiB
and internal free is 21,878,584 KiB; capacity must be checked again at launch.

Compare registration count and missing view IDs first, then tracks,
observations, reprojection statistics **with their report definitions**, and
intrinsic values. The `-001` baseline is 48 views, **46 poses**, 222 tracks,
1,497 observations, with `NP3_042` and `NP3_048` apparently unregistered.
The ablation passes no camera-coverage gate merely by returning exit 0; it
must register all 48 and satisfy the frozen sparse/long-pair gates before any
claim of improvement. If it does, that is only a training-set diagnostic,
not physical accuracy or object-reconstruction validation. Sequential SfM
may have seed/order sensitivity, so a single changed outcome alone does not
prove the intrinsic policy caused it.

License scope remains **evaluation oracle only**: the same binary carries
the unresolved `cmdLine.h` LGPL-vs-upstream-notice conflict, EPL Clp/Osi,
Ceres/Eigen and linked-library questions documented in
[the closure audit](OPENMVG-LICENSE-CLOSURE.md). Running one existing binary
does not resolve commercial/App Store redistribution clearance.
