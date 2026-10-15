# Reference-first reconstruction recovery

2026-09-27. This supersedes further mustard-only parameter ablations as the
immediate execution priority. Existing failures and scores remain unchanged.
The objective is a reproducible, useful reconstruction from ordinary RGB photos,
not a larger sparse cloud or more successful software tests.

The first stock run and an explicit subsequent correction to our overly strict
track-uniqueness gate are recorded in [SCEAUX-STOCK-RESULTS.md](SCEAUX-STOCK-RESULTS.md).
The original pre-run text below is preserved; the dated continuation amendment
does not retroactively change the first report's failed gate flag.

## What established workflows actually do

| Workflow | Relevant upstream behavior | Consequence for this project |
| --- | --- | --- |
| COLMAP → OpenMVS | Extract and verify image features, estimate cameras with bundle adjustment, undistort, then dense reconstruction and meshing | First reproduce this chain with library defaults and explicit resource-only deviations. OpenMVS cannot repair wrong input cameras. |
| Meshroom / AliceVision turntable workflow | Mask feature extraction and dense preparation; documented minimum 2D feature motion suppresses stationary background matches | Masks must affect alignment as well as the final cloud. A fixed pixel-motion cutoff is specific to capture conditions, not a universal rule for moving cameras. |
| Metashape | Distinguishes masks on keypoints from masks on entire tie-point tracks; offers stationary-tie-point exclusion | Background suppression and track consistency are separate from foreground segmentation. Use as workflow reference; no Metashape executable/license is available here. |
| Apple Object Capture | Existing original adapter submits ordinary photos to RealityKit and requests a preview USDZ | Independent M1 product oracle; not the cross-platform shipping backend. Runtime support is proven locally, reconstruction quality is not. |

Sources checked directly: [COLMAP 3.11.1 tutorial](https://github.com/colmap/colmap/blob/3.11.1/doc/tutorial.rst),
[3.11.1 FAQ](https://github.com/colmap/colmap/blob/3.11.1/doc/faq.rst),
[OpenMVS usage](https://github.com/cdcseacave/openMVS/wiki/Usage),
[Meshroom turntable instructions](https://github.com/alicevision/meshroom-manual/blob/develop/source/tutorials/turntable/turntable.rst),
[Agisoft mask guidance](https://agisoft.freshdesk.com/support/solutions/articles/31000158967-aligning-photos-with-apply-masks-to-key-tie-points-option),
[Metashape 2.3 manual](https://www.agisoft.com/pdf/metashape_2_3_en.pdf).
Apple capability/probe evidence is in [APPLE-OBJECT-CAPTURE.md](APPLE-OBJECT-CAPTURE.md).
These are documented behaviors, not measured comparative quality here.

Our recent mustard configuration is **not a stock COLMAP quality baseline**:
it limited SIFT to 1,800 features / 1,200 pixels, fixed heuristic intrinsics,
and later deleted nonlocal match pairs. Those were labeled experiments, but
their failures cannot establish the performance limit of COLMAP/OpenMVS.
Default COLMAP refines focal length/distortion while keeping principal point
fixed; known calibration and heuristic initialization are not interchangeable.

A rigid object can rotate and translate between exposures: cameras can be
expressed in the object's frame. The stationary background then violates that
same rigid-scene model. Neither markers, depth sensing nor exact motor angles
are mandatory inputs. Reflectance changes, weak texture, blur and insufficient
view coverage remain genuine limits; neural rendering does not validate shape.

## Cases and evidence boundaries

| Case | Available input | Reference / role |
| --- | --- | --- |
| P0 Sceaux | 11 original 2832×2128 JPEGs, pinned upstream OpenMVS sample | Supplied OpenMVG cameras and OpenMVS mesh, **evaluation-only software agreement**. Static architecture, not an object benchmark. |
| P1 YCB cracker box | 60 original 1280×1024 turntable JPEGs | Separate Google scanner mesh; Berkeley rig camera metadata kept evaluation-only. Textured development object, not automatically an easy capture. |
| P2 3DLF physical bunny | 73 original 1749×1155 PNG photographs | Separate Revopoint scan including support disk; difficult dark, weakly textured object. |
| Stress / later validation | Mustard failures retained; acquired power drill remains a later object | Do not keep selecting settings solely against mustard or call reused development objects held-out validation. |

Local paths/provenance are documented in [UPSTREAM-CONTROL.md](UPSTREAM-CONTROL.md),
[YCB-COMPARISON.md](YCB-COMPARISON.md) and [BUNNY-EVALUATION.md](BUNNY-EVALUATION.md).
No new large download is needed. Object scans have unverified cross-sensor
transforms; report reference-fitted shape, not calibrated dimensional accuracy.
Bunny support disk and single-elevation YCB missing undersides must be explicit.
Any object-only reference region must be selected from the reference itself
before viewing new outputs; preserve whole-reference results as well.

## Ordered tasks and stop rules

1. **P0 stock sparse control now.** Use pinned PyCOLMAP 3.11.1 on the 11 Sceaux
   JPEGs from scratch. Preserve algorithm defaults; explicitly use CPU,
   two threads and seed 20260927. Camera mode AUTO / SIMPLE_RADIAL, no supplied
   calibration or initial pair, no feature-budget reduction, no masks,
   no track repair and no pair-table edits. Record complete effective options,
   input/core/runner hashes, all model components, timings and failures.
2. **P0 camera check and dense handoff.** After successful sparse processing,
   compare named recovered cameras to supplied software cameras, evaluation-only,
   checking centers and rotations under one proper similarity transform. Check
   finite geometry, unique image observations per track and registration of all
   11 photos before dense continuation. This is eligibility, not shape acceptance.
   If it fails, investigate stock installation/options or input conventions;
   do not compensate with mesh cleanup. Dense continuation gets a recorded
   undistortion/import contract and bounded native settings before execution.
3. **P1 then P2 established full pipelines.** Freeze exact same photo sets and
   mask/input policies for COLMAP/OpenMVS and independent MVE; use normal camera
   self-calibration rather than a fixed guess. Add Apple Object Capture as an
   independent product comparison with its different/internal masking disclosed.
   Native defaults are the baseline; memory/resolution changes are explicit
   variants, not secretly substituted results. One initial configuration per
   pipeline/case, no automatic retry or per-object parameter search.
4. **Locate the first divergence.** Compare cameras before surfaces. On YCB use
   the existing independent rig-camera diagnostic, with convention limitations
   disclosed. Where a reference succeeds and ours fails, replace/fix that stage.
   If both fail, inspect capture/support assumptions before adding more algorithms.
5. **Only then optimize/integrate.** Adopt the demonstrated working route in the
   app; test portability and resources separately. Neural/Gaussian branches wait
   until camera/input validity and a useful classical baseline are established.

P0 sparse budget: one fresh external directory
`/Volumes/backups/code/crispmesh-data/sceaux-stock-sfm-001`, at most 600 seconds,
512 MiB output, sampled child RSS 4 GiB, bounded logs 16 MiB per stage, and at
least 10 GiB free on **both** internal and external volumes. Preserve partial
models on failure. No supplied scene/mesh/camera file enters the worker inputs.
Tests/builds finish before live processing; large data and scratch stay external.
Later native jobs run serially and receive their own budgets before launch.

## Comparison and acceptance

Separate image-estimated end-to-end, supplied-camera density, and composed
recovery runs. Record masks, intrinsics policy, image hashes, preprocessing,
native options, stage completeness, interventions and hardware. Different
native settings are expected across products; report them, and never attribute
a combined pipeline difference to just one numerical stage.

Use registration/failure rate, independent camera agreement where available,
bare-surface inspection, bidirectional surface distances and P/R/F at frozen
0.5/1/2% reference-diagonal tolerances, normal agreement, missing coverage,
and separately elapsed time, peak sampled memory and disk. Keep the existing
frozen alignment/evaluation protocols rather than choosing a favorable fit.
No mesh means unavailable surface scores, not zero error. Better appearance,
lower training residual or closed topology alone cannot pass quality.

The first deliverable is a **reproducible positive control plus honest object
comparisons**, not an arbitrary new percentage target. Product acceptance still
requires recognizable complete object geometry, independent checks and an
untouched-object evaluation. Software-oracle agreement never becomes physical
ground truth. If a baseline cannot reproduce a known successful control, stop
feature expansion around it and resolve that discrepancy first.

GPL/AGPL or platform-specific research comparators remain separate from shipping
approval. Public AGPL licensing does not settle App Store compatibility or
third-party model/data terms; do not redistribute new assets as part of this work.
