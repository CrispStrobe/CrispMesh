# Private Kaggle AliceVision 73-photo bunny: feasibility and stop gates

**Status, 2026-09-28:** plan only. No 73-photo dataset has been uploaded and no
73-photo AliceVision run has started. The three-photo CPU sparse smoke is not
a verified gate: v2 printed a completion line after about 88 seconds but
finished `CANCEL_ACKNOWLEDGED` with no downloadable receipt; the identical
v3 retry also ended `CANCEL_ACKNOWLEDGED` with no receipt or useful worker
log. No further retry is authorized. A working dense/CUDA stage, a suitable GPU, and this account's
remaining GPU allowance have not been established.

## Exact inputs and software boundary

Use only the 73 contrast-prepared PNGs in
`build-opencv/bunny-gamma05-clahe2/`,
totaling **121,809,661 bytes** (116.2 MiB), bound to
`prepare-manifest.json` SHA-256
`eca0bfa60badd7fa5c9b5311e7f413a3f337159c158f856066086e44b0a90ecd`.
The manifest maps `frame_####` to original `bunny_N` names; frame indices
follow lexical source names, **not numeric capture order**. Verify all 73
file sizes/hashes from that manifest immediately before packaging and again
inside Kaggle. Dataset content must be images, manifest, attribution, and
rights note only: no Revopoint scanner, PRO poses, supplied calibration,
prior SfM outputs, masks, or depth maps. Put PNGs under an `images/`
subdirectory or one archive; Kaggle documents a 50 top-level-file limit and
unpacks archives when mounted. Keep the dataset and notebook private. Kaggle
documents private datasets, but privacy is not local-only storage: files
are transferred to Kaggle. The 3DLF-Scan photos are CC BY 4.0 with
attribution to Vodianyk, Nava-Baro, and Popov, DOI 10.17632/ngvgpsvd8b.1;
the physical Stanford-bunny derivative has unresolved commercial rights.
Use this as research-only and do not publish the dataset or textured model.
[Kaggle dataset documentation](https://www.kaggle.com/docs/datasets)
describes private sharing and current dataset limits.

Pin the official [AliceVision v3.3.0 Linux release](https://github.com/alicevision/AliceVision/releases/tag/v3.3.0):
`AliceVision-3.3.0-Linux.tar.gz`, 1,505,191,867 bytes, SHA-256
`f43f498312859af627f2f7f65a6d33c2a3411b37989b8b680c04c8c690dcb640`.
The tested tar inventory has 2,307 members and 2,344,265,238 regular-file
bytes. Use its bundled `aliceVision/bin`, set `ALICEVISION_ROOT` to the
extracted `aliceVision` directory and `LD_LIBRARY_PATH` to its `lib`. Prior
private CPU loader probes loaded `cameraInit` and `depthMapEstimation` help
without missing libraries, but their help exits 1 by design; no real dense
command was tested. The official v3.3.0 source says
`aliceVision_depthMapEstimation` needs a CUDA-capable GPU. Its release notes
include a turntable-object pipeline, but this pinned AliceVision CLI tarball
does **not** itself prove that a corresponding Meshroom turntable graph is
installed. Freeze and inspect the matching graph/template and every actual
command before claiming a turntable-pipeline run. A plain CLI photogrammetry
chain must be labeled separately.

## Stage interfaces to pin before a full run

The existing three-photo script verifies and runs the first five executables:
`aliceVision_cameraInit --imageFolder --sensorDatabase --defaultFieldOfView
--output`; `aliceVision_featureExtraction --input --describerTypes sift
--forceCpuExtraction true --output`; `aliceVision_imageMatching --input
--featuresFolders --minNbImages --output`; `aliceVision_featureMatching
--input --featuresFolders --imagePairsList --describerTypes --output`; and
`aliceVision_incrementalSfM --input --featuresFolders --matchesFolders
--describerTypes --output --outputViewsAndPoses --extraInfoFolder`. Their
actual v3.3.0 help flags and real output files must match the verified
three-photo receipt before scaling to 73.

For the dense half, the tagged source specifies these required handoffs:

| Executable | Required v3.3.0 interface | Output gate |
| --- | --- | --- |
| `aliceVision_prepareDenseScene` | `--input` SfMData, `--output` folder | Nonempty undistorted images for resolved views; source defaults to EXR. |
| `aliceVision_depthMapEstimation` | `--input` SfMData, `--imagesFolder` prepared images, `--output` folder | GPU identified, CUDA probe passes, nonempty depth maps; support `--rangeStart/--rangeSize` pilot and fixed `--downscale`. |
| `aliceVision_depthMapFiltering` | `--input` SfMData, `--depthMapsFolder`, `--output` folder | Nonempty filtered depth maps, with view IDs matching the calibrated set. |
| `aliceVision_meshing` | `--input` SfMData, `--output` dense SfMData, `--outputMesh` mesh; supply `--depthMapsFolder` | Finite nonempty dense model and triangle mesh; without the depth folder it may mesh sparse SfM alone. |
| `aliceVision_meshFiltering` | `--inputMesh`, `--outputMesh` | Nonempty valid filtered mesh; no post-hoc parameter tuning to a scanner score. |
| `aliceVision_texturing` | `--input` dense SfMData, `--inputMesh`, `--output` folder; prepared `--imagesFolder` as needed | Mesh, material, and bounded texture files all present and loadable. |

These interfaces come from the [tagged prepare source](https://raw.githubusercontent.com/alicevision/AliceVision/v3.3.0/src/software/pipeline/main_prepareDenseScene.cpp),
[depth source](https://raw.githubusercontent.com/alicevision/AliceVision/v3.3.0/src/software/pipeline/main_depthMapEstimation.cpp),
[filter source](https://raw.githubusercontent.com/alicevision/AliceVision/v3.3.0/src/software/pipeline/main_depthMapFiltering.cpp),
[meshing source](https://raw.githubusercontent.com/alicevision/AliceVision/v3.3.0/src/software/pipeline/main_meshing.cpp),
[mesh-filter source](https://raw.githubusercontent.com/alicevision/AliceVision/v3.3.0/src/software/pipeline/main_meshFiltering.cpp),
and [texturing source](https://raw.githubusercontent.com/alicevision/AliceVision/v3.3.0/src/software/pipeline/main_texturing.cpp).
Capture each extracted binary hash and its actual `--help` interface in the
future runner; the table is a design contract, not a claim that dense stages
have run on Kaggle.

## Bounded execution proposal, conditional on approval

1. **Three-photo receipt and privacy gate.** Require a complete, downloadable
   CPU sparse receipt, exact image hashes, three imported views, nonempty
   matches and SfM model, and no scanner/pose/calibration inputs. Resolve
   source-dataset transfer approval and confirm private dataset/notebook
   metadata before staging the 73-photo package. This gate currently fails;
   stop here unless a new, explicitly reviewed path yields durable evidence.
2. **Read-only account check, then GPU capability pilot.** The Kaggle CLI
   used here has no remaining-GPU-quota endpoint; the CPU notebook had no
   `nvidia-smi`. Kaggle says GPU quota/availability varies and can be checked
   in its notebook/account UI. Do not infer remaining hours from the generic
   documentation. Once explicitly approved, use a short private GPU-enabled
   pilot to record GPU model, driver, VRAM, CUDA compatibility, free scratch,
   account allowance and a real depth-map result on a bounded subset. Stop
   if no GPU, insufficient quota, incompatible CUDA, or depth failure.
3. **73-photo sparse gate before dense.** Fresh private root, manifest-hash
   input validation, explicit commands and capped logs. Suggested total
   sparse wall cap **90 minutes** and process RAM cap **24 GiB**, adjusted
   downward to observed GPU/CPU session RAM. Require 73 named input views,
   a substantial resolved-view set, geometric tracks, camera orbit/scale
   plausibility, and object-focused neutral previews. In particular, a
   73/73 pose count alone is not enough: the previous OpenMVG control
   registered all views yet its rough mesh was dominated by background.
   Fail closed on background-lock or poor object geometry; do not run dense.
4. **GPU dense and mesh gate.** Run prepared images and a small fixed depth
   range first; validate hashes, view IDs and visual depth evidence, then
   only proceed to full ranges. Suggested upper caps: **3 hours** for all
   depth estimation/filtering, **1 hour** for meshing/filtering, and **30
   minutes** for texturing; **6.5 hours total** including sparse and overhead.
   Enforce per-stage timeout, output-size and log caps, process-tree memory
   bound, persistent receipt after any failure, and no automatic repeat.
   Stop immediately on a cap, missing artifact, geometry failure or bad
   neutral shape. Do not present a textured result as successful merely
   because a command exited zero.
5. **Download only sealed review artifacts.** Keep raw outputs ephemeral and
   durable `/kaggle/working` output to at most **1 GiB** (mesh, modest
   texture, previews, capped logs, hashes and JSON). A human-visible neutral
   XY/XZ/YZ geometry review precedes any scanner-fit diagnostic. Scanner
   fitting is a caveated reference-fit measure, not an independent ranking.

The pinned archive plus actual extracted files and 73 PNGs need about
**3.97 GB** before intermediate outputs. A proposed **8 GiB work cap** plus
a **4 GiB free-space floor** requires at least **~16 GiB free scratch** at
start (round upward for filesystem overhead); the per-session free-space
check must supersede this estimate. At 1,749×1,155 pixels per image, 73
uncompressed RGB float EXRs alone could approach 1.8 GB, and multiple depth
products can add several GB. Thus 8 GiB is a resource ceiling, **not** a
promise that the complete graph will fit. Kaggle documents 20 GB autosaved
`/kaggle/working` space and additional ephemeral scratch, but current
session capacity must be measured. The 6.5-hour time envelope is a proposed
stop bound, not a runtime prediction. [Kaggle notebook documentation](https://www.kaggle.com/docs/notebooks)
describes session limits and variable accelerator availability.
