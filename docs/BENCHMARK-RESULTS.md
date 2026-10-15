# Comparative reconstruction benchmark

This is measured development evidence, not a state-of-the-art claim. Different
lanes answer different questions; their scores must not be pooled into a ranking.

## Lanes

| Input | Pipelines | Reference and scope |
| --- | --- | --- |
| YCB cracker box, 60 real rotating-object photos | MVE image-only; COLMAP/OpenMVS raw/masked camera trials; fixed-camera native mask/resolution ablations | Separate Google object scan; whole-surface, reference-fitted shape diagnostics, not independently recovered metric scale |
| 3DLF bunny, 73 real photos | MVE and COLMAP/OpenMVS on byte-identical contrast-preprocessed PNGs | Separate Revopoint physical-instance scan; fitted alignment/scale; raw-photo failures retained |
| Sceaux castle, 11 upstream photos | MVE image-only; upstream OpenMVG cameras followed by local OpenMVS CPU stages | Upstream software mesh and cameras are regression oracles, **not ground truth**; supplied-camera and image-only lanes remain separate |

Primary shape metrics remain precision, recall and F at 0.5%, 1%, and 2% of
reference bounding-box diagonal. Additional diagnostics report both directed
distance distributions, unsquared symmetric mean normalized by diagonal,
orientation-insensitive normal agreement, boundaries, nonmanifold edges and
components. See [the exact metric protocol](COMPARATIVE-BENCHMARK.md).
Low error on a small retained fragment is not success: completeness and failures
must accompany it. Raw camera registration and reprojection error are not surface
quality metrics. Face count is not accuracy. Texture is not used to hide defects.

## Current findings

The latest [sensor-depth benchmark](SENSOR-DEPTH-BENCHMARK.md) adds a
stage-matched rough-mesh comparison over 6,144 fixed observed-supported rays
in three views, without mesh-to-reference fitting:

| Rough configuration | Predicted-ray coverage | Hit-only mean absolute depth disagreement | Within 5 mm, including misses in denominator |
| --- | ---: | ---: | ---: |
| Earlier image-only cameras, native masks (`008`) | 84.08% | 2.88 mm | 70.72% |
| Supplied intrinsics, image-estimated poses (`014`) | 83.66% | 5.96 mm | 38.56% |

The fixed eroded-interior check also favors `008` on residuals (2.34 versus
5.64 mm), although `014` has slightly higher interior ray coverage (94.22%
versus 92.58%). These are assumption-qualified, same-capture sensor diagnostics,
not whole-object accuracy or metrology. They conflate geometry and camera/reference
pose errors, and the two configurations differ in verification and downstream
settings. No calibration-only causal claim follows. The calibrated continuation
completed rough meshing but timed out in refinement; no refined-pair result exists.

**Registration audit qualification:** the frozen known-transform
[audit](REGISTRATION-AUDIT.md) now demonstrates that our existing surface
aligner can misorient independently sampled copies of the *same* scanner mesh
by 16.67–25.25 degrees. Missing support and outliers can make its objective
prefer an incorrect transform. Historical scores below are preserved, but
they are not trustworthy intrinsic pipeline rankings or independently
registered physical accuracy. Camera-frame and sensor-depth controls are the
new diagnostic lane above; this finding does not establish that the underlying
reconstructions are good.

A [three-frame Berkeley sensor-depth inspection](BERKELEY-DEPTH-FEASIBILITY.md)
confirms that the already-local archive contains usable 640 × 480 raw depth
arrays and distinct depth-camera calibration (under 2 MB extracted). That
initial feasibility report contained no scores; the subsequent qualified
projection check and depth comparison above supersede that status. Pixel
conventions, distortion and object-only visibility retain explicit uncertainty.

The [fixed-intrinsics camera experiment](CALIBRATION-ABLATION.md) retains a
threaded-verification crash and a serial image-only 2/60 failure. Its preselected
serial calibrated arm recovers 60/60 cameras and 3,942 sparse points, with
median camera-orientation disagreement 1.34 degrees and center RMS 0.00848 in
Berkeley native units. This supplied-calibration result is not an ordinary-photo
success or a paired mesh-quality improvement. Its first
[native dense control](CALIBRATED-CONTROL.md) exceeded the 1 GiB cap. The later
resolution-corrected dense run and rough-mesh continuation supply the new
sensor comparison above; refinement remains incomplete.

YCB fixed-camera comparisons (same 60 estimated cameras; 2,048 query samples
per direction, independent reference fits unless stated):

| Variant | F at 1% diagonal | Normalized mean distance (lower is better) | F using unchanged 006 alignment |
| --- | ---: | ---: | ---: |
| Unfiltered classical 004 | 12.77% | 5.704% | 18.98% |
| Photo-support cloud filter 006 | 38.15% | 2.525% | 38.15% |
| Native depth masks 008, 641 × 512 | 42.04% | 3.053% | 35.49% |
| Native depth masks 011, 1282 × 1024 | 42.06% | 3.128% | 35.52% |

The 011 result is a cache continuation after 009 exceeded its 2 GiB output
budget, not a fresh uninterrupted run. Both native-mask meshes have one closed
component and no counted nonmanifold edges, yet poor reference-fit agreement.
More depth pixels and watertight topology did **not** improve that diagnostic;
the subsequent registration audit prevents attributing all its error to shape.
The independently fitted and shared-alignment columns answer different questions;
neither is independently calibrated physical accuracy. See
[alignment sensitivity](SHARED-GAUGE.md).

- YCB cloud filtering improved 1% F from 12.77% to 38.15%, normalized mean distance
  from 5.704% to 2.525%, and normal agreement. It remains rejected quality.
- The pre-refinement YCB reference-fit scores were low (1% F 14.95% unfiltered,
  33.87% filtered). Because registration is unreliable, these scores alone
  cannot locate a geometry defect or determine refinement's contribution.
- The first COLMAP bunny run stopped at a two-view model. A generic mapper-only
  retry allowing up to five models found a second model with all 73 views and
  3,133 points in 22.41 seconds. Root independently loaded the saved model: mean
  reprojection error 0.663 pixels. Returned model indices and saved directory
  indices differ; the verified 73-view model is `models/0`, not `models/1`.
  This is a composed retry, not a new uninterrupted end-to-end success.
- MVE completed the Sceaux image-only pipeline with 11/11 cameras and 41,339
  mesh faces in 135.96 seconds. Its first surface-fit comparison to the upstream
  software mesh has weak agreement and did not converge; camera-based alignment
  is a separate diagnostic, not an opportunity to hide the original result.
- Local OpenMVS reconstructed 98,954 dense points and 187,793 rough faces from
  the supplied Sceaux camera scene. Rough-mesh agreement to the upstream software
  mesh is 94.04% F at 1%; this is a positive numerical integration control, not
  evidence of 94% object accuracy. A mesh-file handoff failure was preserved,
  then a hash-bound continuation completed refinement and texturing.

An independent camera diagnostic compares the YCB trajectory with the supplied
Berkeley rig metadata, without feeding those poses into reconstruction. Median
orientation disagreement is about 2.48 degrees; fitted center RMS is 0.00872 in
the metadata's native translation units, for an approximately 0.488-radius
orbit. Estimated focal length is close to the supplied value, but the fixed
principal point differs. This narrows the diagnosis; it does not certify the
camera model or align the separate Google mesh. See
[reference-frame caveats and reproduction](YCB-REFERENCE-FRAMES.md).

No missing mesh receives a fabricated zero surface score: report the failure
and registration count instead. The unmasked bunny continuation completed
refinement and texturing (42,919 refined faces). Its whole-scan F is 17.73%,
versus 16.65% for the rough mesh and 29.47% for MVE. **These numbers cannot
establish an intrinsic geometry ranking:** the scanner includes a large support
disk, and the classical reference-fit alignment visibly fails. A separately
frozen, post-hoc scanner-only object ROI diagnostic is therefore evaluated on
every candidate, retaining all candidate triangles and the original scores.
See [bunny comparison and evaluation caveats](BUNNY-CLASSICAL-COMPARISON.md).

The post-hoc ROI did **not** solve registration: at 1% of its own diagonal,
F is 20.68% for MVE, 13.13% for classical rough and 13.99% for classical refined.
MVE's high recall accompanies diffuse extra geometry and poor normals; the
classical outputs look bunny-like but are visibly misoriented against the scan.
These are recorded diagnostic outcomes, not a defensible winner/loser ranking.
Independently validated registration is now an evaluation prerequisite.

Native bunny masks also have direct payload evidence: all 73 depth maps retain
4,332,340 positive inside-mask depths and zero outside, versus 4,332,360 inside
and 4,436,808 outside in the unmasked baseline. Dense runtime was 307.25 versus
897.53 seconds in these observed trials. This supports correct mask application
and a promising cost reduction, not a controlled speedup or geometry-quality claim.
The masked fixed-camera chain completed through textured OBJ export: 294,257
fused points, 41,283 refined faces, 806 MB output, with source hashes unchanged.
Both masked and unmasked classical completions reuse the recovered camera model;
neither is a fresh uninterrupted automatic photo-to-textured-mesh replay.
Masked refined F is 17.40% against the whole scan, 19.92% at 1% of the object
ROI's diagonal, and 32.36% on that ROI using the original whole-scan absolute
tolerance. These are distinct evaluation questions, not interchangeable scores.
The ROI normalized mean is 3.547%, versus 4.738% unmasked, and the output has
one component instead of six. However, registration/coverage disagreement remains
visible; these diagnostic changes do not establish accepted geometric accuracy.

## What the evidence says to do next

1. Fix camera initialization robustness: the generic retry recovered the bunny,
   but the box still needed an explicitly selected image pair for a fresh
   60/60-camera replay. That replay is a sparse-only, manually assisted result.
2. Diagnose object geometry with fixed cameras, silhouette/depth overlays and
   camera-model ablations. The box resolution experiment gives no reason to
   spend substantially more compute on the same settings. Its principal point,
   distortion model, mask semantics and reconstruction support need isolated
   tests, not simultaneous tuning against the scanner mesh.
3. Repeat successful settings on another object before product promotion. Keep
   physical-scan comparisons separate from the positive Sceaux software control.
4. Run serial, matched-resolution performance trials and cross-platform real-photo
   tests after quality improves. Current contended timings and build CI cannot
   establish speed leadership or Windows/Linux reconstruction quality.

Neural/GPU methods and a direct KIRI comparison remain unrun. Nothing in these
measurements establishes KIRI parity or state-of-the-art quality.

## Resources and reproducibility

This batch runs on Apple M1 CPU, with two native threads per reconstruction,
bounded deadlines/output sizes and a 10 GiB free-disk reserve. Some jobs overlap:
recorded wall times are observed costs, **not a controlled speed ranking**. MVE
and OpenMVS resolution/mesh settings also differ. Serial repeats on a shared
profile would be required for comparative performance claims.

Selected observed stage costs (seconds; different profiles, no speed ranking):

| Trial | Dense reconstruction | Meshing | Refinement | Texturing | Qualification |
| --- | ---: | ---: | ---: | ---: | --- |
| YCB native masks 008 | 56.97 | 2.60 | 122.47 | 6.27 | Previously computed 60-camera scene; 641 × 512 depth |
| YCB full-resolution continuation 011 | 111.02 | 7.85 | 101.71 | 6.17 | Dense figure excludes cached initial depth work; both geometric passes rerun |
| Sceaux OpenMVS CPU | 128.30 | 10.00 | 102.36 | 22.08 | Supplied upstream cameras; recovered refinement/texturing after a handoff failure |
| Bunny unmasked classical | 897.53 | 21.10 | 236.08 | 35.43 | Cached recovered cameras; successful refinement/texturing continuation after timeout |
| Bunny native masks 005 | 307.25 | 18.57 | 262.46 | 35.52 | Same cached cameras and dense profile; new native mask arm |

Sceaux MVE's image-only pipeline took 135.96 seconds across all its stages, but
its geometry disagrees substantially with the software oracle. Faster completion
of that profile is not evidence of equivalent quality at lower cost. Native RSS
reports measure a sampled child process, not a simultaneous whole-process-tree
memory total. Each linked native report retains its resource limits and failure
state; aborted attempts are not silently subtracted from development cost.

Original photographs, failed outputs and each fresh variant are preserved in
ignored local directories; no datasets or meshes are committed. Hashes, commands,
settings, native exits and validation results accompany runs. Benchmark-only
dependencies are not automatically approved for App Store distribution.

Details: [YCB](YCB-COMPARISON.md), [bunny](BUNNY-EVALUATION.md),
[classical backend](CLASSICAL-BACKEND.md), [upstream control](UPSTREAM-CONTROL.md),
[software-oracle comparison](UPSTREAM-COMPARISON.md).

## Verification checkpoint

Local checks: 245 Python tests (8 skipped), 11 native CTests, 30 desktop tests,
43 CLI/web contract cases, web production build, live Chromium interaction test,
and the 530-entry locked dependency inventory check passed. At code commit
`0f470ad`, both the [quality harness](https://github.com/CrispStrobe/CrispMesh/actions/runs/36287594304)
and [foundation matrix](https://github.com/CrispStrobe/CrispMesh/actions/runs/36287594254)
passed on macOS, Windows and Linux. Photo reconstructions in this report ran on
M1 only. No native UI, mobile packaging, GPU or KIRI comparison is implied by CI.
