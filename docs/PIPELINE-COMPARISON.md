# Pipeline comparison: current CrispMesh and three reference pipelines

**Historical assessment below.** The 2026-09-27 classical integration has since
produced M1 meshes through recorded recovery stages; OpenMVS is now installed
and exercised, but quality and fresh-run reliability remain rejected. See
[YCB-COMPARISON.md](YCB-COMPARISON.md) and the active
[comparative execution plan](SOTA-ROADMAP.md). The old capability table must not
be interpreted as current installation status or a measured quality ranking.

Latest local diagnostic: the clean 1,773-point observation-holdout model is
mostly background under manually inspected tree envelopes (151 points have
all observations inside). Restricting training feature centres to those
envelopes yields 5/10 registered images and worsens missing-inclusive errors
on the same 82 foreground held-out pairs from 8 to 23. This is a rejected
sparse ablation, not an improved scan or a benchmark ranking of COLMAP.
See [foreground support](FOREGROUND-ROI.md) and
[the fixed-population experiment](COLMAP-FOREGROUND.md).

Assessment date: 2026-09-26. This is a capability and test-readiness estimate, **not a measured ranking of finished scans**. The three reference pipelines have not been run on a common CrispMesh capture in this assessment. Product documentation establishes available stages, not accuracy on our objects. The local [status](STATUS.md), [stereo oracle results](ORACLES.md), [profile results](STEREO-PROFILES.md), and [real-tree attempt](TREE-DENSE.md) establish what CrispMesh has actually demonstrated.

| Pipeline | Current end-to-end capability | Platform and compute in official documentation | Evidence here / estimate |
| --- | --- | --- | --- |
| CrispMesh | Calibrated marker pose and masked sparse tracks in the optional native build; stereo, rectification, MVE dense, fusion and bundle adjustment remain test or research paths. No accepted photo-to-mesh scan. | Native core and host verified on Apple Silicon macOS; portable targets are planned, not proven full-scan platforms. | Real finished-scan quality **unknown**. Production readiness for end-to-end photogrammetry is low: real-tree dense output was effectively empty and no full scan or mesh has passed acceptance. |
| COLMAP + OpenMVS | COLMAP supplies image feature matching, camera reconstruction and sparse structure; OpenMVS takes cameras and sparse points through dense cloud, mesh reconstruction, refinement and texture. [COLMAP overview](https://colmap.github.io/), [OpenMVS README](https://github.com/cdcseacave/openMVS). | COLMAP builds on Linux, macOS and Windows; its own dense path is constrained by GPU support, so the relevant comparison is COLMAP sparse + OpenMVS dense. OpenMVS has CPU stages and optional CUDA acceleration; platform builds include Windows, Ubuntu and macOS. [COLMAP install](https://colmap.github.io/install.html), [COLMAP GPU FAQ](https://colmap.github.io/faq.html), [OpenMVS architecture](https://github.com/cdcseacave/openMVS/blob/develop/docs/architecture.md), [OpenMVS README](https://github.com/cdcseacave/openMVS). | **Full pipeline untested.** Corrected PyCOLMAP sparse-only baselines registered 9/10 reduced images with 2,369–2,376 points; original JPEGs yielded 7 cameras but zero retained points. These are training results, not independent accuracy. The first custom holdout trial is invalid. OpenMVS remains uninstalled/unrun. |
| Agisoft Metashape Professional 2.3 | Documented workflow covers alignment, point cloud, mesh and texture; masks, markers and scale bars support isolated, physically scaled objects. [2.3 manual](https://www.agisoft.com/pdf/metashape-pro_2_3_en.pdf). | Official manual lists Windows, macOS and Debian/Ubuntu; the GPU guidance lists OpenCL or CUDA acceleration and CPU/Apple Silicon configurations. Check the exact machine and license before a future run. [2.3 manual](https://www.agisoft.com/pdf/metashape-pro_2_3_en.pdf), [system requirements](https://www.agisoft.com/index.php?id=49). | **Untested here.** A mature complete workflow makes it a strong practical reference, but no relative geometry, coverage, speed or reliability number is available for our capture. |
| RealityScan desktop 2.2 | Documented image alignment, scaled control or distance constraints, reconstruction, texturing and export. [scaling](https://rshelp.capturingreality.com/en-US/tutorials/scaling.htm), [reconstruction commands](https://rshelp.capturingreality.com/en-US/tutorials/commandline_2.htm). | Primarily Windows; Linux is available but recommended for CLI workflows, with UI limitations. No supported macOS desktop path is documented. A supported GPU is needed to create models and textures; 2.2 adds supported AMD GPUs **on Windows**, alongside NVIDIA. General requirements still describe CUDA and should not be read as the complete 2.2 Windows GPU list. [requirements](https://dev.epicgames.com/documentation/realityscan/hardware-and-software-requirements), [2.2 release notes](https://dev.epicgames.com/documentation/realityscan/realityscan-2-2), [Linux limits](https://dev.epicgames.com/documentation/realityscan/known-issues-and-limitations-linux). | **Untested here.** A complete commercial reference where supported hardware is available; relative quality on our capture is unknown. |

The qualitative gap is therefore clear in **workflow maturity**, but uncertain in **surface accuracy**. The three references document complete mesh workflows; CrispMesh has no accepted full-scan output to score. That is a low-confidence expectation about likely end-to-end readiness, not evidence that any reference will achieve a specified millimetre error, percentage advantage, or even a successful scan on a particular object. Foliage, weak texture, reflections, masks, view overlap, scale anchors and camera calibration can dominate the result in any pipeline.

## What our measurements actually compare

Correction from the upstream API audit: the initial PyCOLMAP `run-001` is
**not a valid held-out benchmark**. Its wrapper treated six-column affine
keypoints as position/scale rows, splitting duplicate orientations across
training and validation. It also supplied a guessed focal length through an
API that marks it as a known prior. Its 3/10 registration count describes
that custom experiment, not a fair stock-COLMAP capability test. Standard
full-feature baselines are tracked in [COLMAP-SPARSE.md](COLMAP-SPARSE.md).

Those corrected reduced-image baselines registered **9/10 cameras and
2,369–2,376 points** in two fresh runs, with model consistency checks passing.
The original-JPEG baseline registered 7 cameras but retained zero points.
This supersedes the initial custom sparse-trial result, without advancing
the full COLMAP + OpenMVS lane to “tested.” Training residuals are not
independent geometry accuracy, and the two reduced runs were not bitwise
identical despite the same seed.

The existing 12-run [Middlebury matrix](ORACLES.md) compares OpenCV SGBM, external libELAS and a project-authored Census matcher on four **rectified stereo pairs**, scored against independently supplied disparity truth. Same-scene matcher inputs and truth are checksummed. This is a depth-matching diagnostic; it excludes camera estimation, turntable geometry, object isolation, fusion, meshing and texture. ELAS interpolates to full reference coverage, so coverage alone is not proof that every pixel is independently supported. Census is a simple reference matcher, not a commercial pipeline proxy.

The later `fast` SGBM_3WAY profile and ELAS used byte-identical prepared left/right/truth files in all four scenes (the SHA-256 values in `build-opencv/stereo-profiles/run-20260926T110550Z-6277acf1/summary.json` match `build-opencv/benchmarks/run-20260926T105252Z-5527a422/benchmark.json`). Piano's prepared search count of 65 is rounded to the same 80 candidate disparities for both. `fast` values below are medians of five serial repeats after warmup; ELAS values are one fixed oracle-matrix run. `bad2 all` counts missing estimates as bad among valid truth pixels. These are compatible **quality** comparisons on the same four inputs, but not a controlled timing comparison across the separate runs.

| Scene | `fast` coverage | ELAS coverage | `fast` bad2 all | ELAS bad2 all |
| --- | ---: | ---: | ---: | ---: |
| Cones | 75.22% | 100.00% | 28.51% | 10.76% |
| Piano | 84.34% | 100.00% | 30.21% | 18.36% |
| Teddy | 75.09% | 100.00% | 29.69% | 15.04% |
| Venus | 77.34% | 100.00% | 23.78% | 3.70% |

The `fast` profile improved time and missing-inclusive error over the project's **baseline SGBM** on these development scenes; it has not been selected as the production backend. ELAS's lower missing-inclusive error here does not predict mesh quality from a moving camera or turntable. The first real-tree selected-MVE attempt returned **0 and 2 valid depth pixels**; a fresh scene with the corrected COLMAP-to-OpenCV principal-point convention returned the same counts. A tree bundle-adjustment candidate was rejected: only 9 of 423 tracks spanned three views, reserved-observation RMS worsened from 574.5 to 839.3 px despite training RMS improving from 1.029 to 0.606 px, and the two dense views still yielded only 7 and 0 valid pixels. A separate fixed-pair test on the corrected cameras produced nonempty image-space disparity: SGBM covered 21.31% of pixels and ELAS reported 100% finite output including filled regions. At weak held-out sparse correspondences, median absolute disparity difference was 0.59 px for 26 SGBM-valid points and 1.07 px for 36 ELAS-valid points; this is not depth ground truth or a usable tree reconstruction. The synthetic board-anchored bundle-adjustment candidate improved a finite-panel surface diagnostic but was **rejected** because marker RMS increased beyond its preset safeguard. That diagnostic is not measured real-scan accuracy. [Tree run](TREE-DENSE.md), [tree refinement](TREE-REFINE.md), [tree pair oracle](TREE-PAIR-ORACLE.md), [synthetic bundle experiment](BUNDLE-QUALITY.md).

## Protocol for a meaningful head-to-head

1. Freeze one capture before trying a reference pipeline: raw original photos, lens/intrinsic evidence, unchanged image ordering, object masks and their provenance, marker-board geometry, independent measured scale objects, and documented lighting, camera and turntable motion. Use a second, held-out capture for any winner claim. Do not tune on the final scoring object.
2. Run a **camera-estimated lane** from the same original photos, with comparable allowed masks, anchors and scale. Record image registration, pose and intrinsic estimates, scale residuals, failures and intervention time. This measures the full workflow. Report unsupported platform or unavailable GPU as an unavailable lane, not a zero-quality score.
3. Run a **fixed-camera dense lane** only where a verified pose/intrinsic import is supported. Give each dense engine the same calibrated cameras, image pixels, mask semantics and region of interest. Verify coordinate systems and pixel-center convention with reprojected observations before measuring surfaces. This isolates dense reconstruction from camera estimation. Keep it separate from the end-to-end lane; do not assume every product can accept exactly the same camera constraints.
4. Score all outputs with an **independent physical reference**: a calibrated scan, machined/test object, or measured surface and scale that no optimizer saw. Align with a declared rigid/scale policy, keep an untouched check region, and report completeness/coverage, matched-surface distance distribution, boundary error, gross outliers, scale error and failure rate. Include timing, peak memory, machine/GPU, software version, settings and any manual cleanup. Show a missing-inclusive metric so an almost-empty but locally accurate mesh cannot win.
5. A reconstruction from any comparator may be useful as **pseudo-ground truth for visual debugging**, but it is not an independent accuracy reference. Agreement with Metashape, RealityScan, COLMAP/OpenMVS or MVE only shows agreement; it cannot establish physical millimetre accuracy. Keep the Middlebury disparity truth, synthetic renderer truth and measured real-object truth as separately labeled evidence.

COLMAP's core license is described as BSD, with third-party dependencies subject to their own terms; OpenMVS declares AGPL-3.0. The proposed COLMAP + OpenMVS combination is therefore only a separate research oracle here, **not a shipping dependency or an integration approval**. License notes are engineering scoping, not a legal guarantee. [COLMAP license](https://colmap.github.io/license.html), [OpenMVS license](https://github.com/cdcseacave/openMVS/blob/develop/LICENSE). Only isolated PyCOLMAP sparse reconstruction has since been installed and run; no full alternative pipeline or external account was used. See [the bounded sparse experiment](COLMAP-SPARSE.md) and [independent verification](SPARSE-ORACLE-VERIFY.md).
