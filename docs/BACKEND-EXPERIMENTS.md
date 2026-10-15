# Backend experiments: evidence before selection

For the current execution inventory, three-branch plan and additional data queue,
use [PIPELINE-COMPARISON-PLAN.md](PIPELINE-COMPARISON-PLAN.md). The dated proposal
below predates the completed local COLMAP/OpenMVS runs and must not be read as
their current execution status.

2026-09-27: the approved [quality-first roadmap](SOTA-ROADMAP.md) supersedes the
ordering and oracle-only OpenMVS restriction in this historical proposal. Original
project code is now AGPL-3.0-only; desktop/server integration is a candidate, while
third-party rights and App Store review remain separate. Retain the experiment
contract below; do not mistake candidate algorithms for implemented features.

Plan dated 2026-09-26. This proposes research comparisons; none of the new
backends below has passed a same-input, measured-object scan comparison or a
shipping review. Continue to use the [quality workstream](QUALITY-IMPROVEMENT.md)
and [dependency policy](DEPENDENCIES.md) as the gates. KIRI Engine's range of
scan outputs is useful product inspiration. Its [2024 announcement](https://www.kiriengine.app/blog/announcement/3dgs-to-mesh-convert-visualizations-to-obj)
credits a 3DGS-to-mesh collaboration with researcher Chongjie Ye; its [2026
release](https://www.kiriengine.app/blog/kiri-engine-4.2-release) announces
Mesh 3.0. Neither establishes that the public GauStudio repository is KIRI's
current production implementation. Do not use that inference to select it.

## Common experiment contract

Freeze image files/hashes, capture settings, intrinsics, object masks, splits,
scale evidence, time/memory/output limits, and scoring populations before each
comparison. Fit from **images only**: measured reference geometry, laser points,
reference depths, supplied SfM seeds and held-out observations may score a run
but may not initialize, tune or repair it. If an algorithm needs sparse seeds,
derive them from the permitted training photos and record their provenance.
Retain failures, rejected observations, empty outputs and unregistered cameras.
Use a fresh, licensed measured turntable object for acceptance; ETH3D pipes and
the current tree are development cases, not unseen acceptance scenes.

Run two explicitly labeled camera lanes on identical training photos and masks:

| Lane | Input to reconstruction | Question answered |
| --- | --- | --- |
| Supplied-camera oracle | Supplied intrinsics/poses with validated conventions and documented uncertainty; image-derived object matches or depths only | How do matching, density and meshing behave with cameras fixed to the reference? This does not prove zero camera error. |
| Estimated-camera end to end | Cameras and object evidence estimated from training images, with board observations and measured calibration where the product permits | What can this scanner actually deliver? |

Do not combine a supplied-camera surface score with estimated-camera runtime or
call an oracle-pose result an application improvement. A rotating board needs a
per-frame object-to-camera transform: reject the stationary-background solution
as an object pose, validate scale and pose handedness/projection, keep board and
background pixels out of object geometry, and test outside-mask image mutation.
If a backend requires observed tracks, construct a verified image-derived
sparse scene; do not invent tracks from bare camera rows. Missing physical
scale requires separate calibration or a known reference, not merely more
image matches. Supplied cameras refined against the reference scan must be
disclosed as such, not described as independent pose ground truth.

Score **surfaces** against independent geometry in the declared physical units:
visible-surface accuracy, completeness/coverage, silhouette and thin-feature or
depth-step errors, plus a fixed-population missing-inclusive bad-or-missing rate.
Report matched-only error and the number discarded beside the fixed denominator;
inpainted or smoothed regions are not observed coverage. Separately score
**rendering** on held-out views (photometric error and silhouette, with exposure
and view dependence documented). A good novel-view render is not evidence of an
accurate mesh. Record camera registration, scale error, runtime, peak memory and
output size for each lane. Do not rank physical surface accuracy on the tree,
which lacks independently measured geometry and verified metric scale.

## Ordered work

1. **Now: correspondence robustness.** On the existing development images,
   predeclare ambiguity and third-view/cycle checks before rerunning. Diagnose
   repeated features, epipolar residuals, cheirality, parallax and mask-edge
   failures; preserve original match IDs, held-out groups and all rejections.
   Compare the same reserved observations under supplied and fitted cameras.
   Advancement requires better fixed-population bad-or-missing behavior without
   unacceptable loss of views or object support; a lower residual among
   survivors is insufficient. This follows the [pipes outlier](PIPES-SCORE.md)
   and [tree failure](TREE-TRACKS.md) evidence.
2. **Next: isolated CPU full-pipeline oracle.** Once a VPS capability, storage
   and data-use preflight succeeds, pin COLMAP and OpenMVS versions, executable
   hashes, flags and licenses. On the same small original-photo set, run COLMAP
   CPU feature extraction/matching and mapping, then verified sparse conversion,
   OpenMVS density and mesh stages under resource limits. Keep OpenMVS AGPL-3.0
   as a separate research executable with no app linking, bundling or runtime
   requirement; this requires the policy exception to be explicitly confirmed
   before execution because the current dependency note only explicitly permits
   GPL test oracles. The [readiness check](PIPELINE-ORACLE-READINESS.md) found no
   local executables, so no full-pipeline result exists. Report failures and
   native mesh output even without ground truth, but do not claim a quality
   winner until independent measured geometry exists. CPU availability follows
   COLMAP's documented CPU flags and OpenMVS's optional CUDA build setting;
   feasibility on this VPS remains to be measured.
3. **Build the reusable core: confidence-aware depth fusion.** Extend the
   existing conservative image-derived pair-to-object-frame path rather than
   inferring confidence from a single low residual. Calibrate confidence from
   image-only support, left/right agreement, multi-view reprojection, parallax,
   local texture, boundary proximity and pose uncertainty; preserve raw depth,
   confidence and rejection reasons. Fuse consistent, visible observations
   across distinct camera centers with explicit occlusion and mask checks.
   Compare raw versus fused output on known nonplanar/depth-step fixtures and
   measured objects using the same fixed denominator. Define a common record:
   metric depth, depth convention (camera `z` versus ray range), calibrated
   camera, object mask, calibrated confidence and source/provenance (measured,
   MVS, learned or rendered). Convert conventions before fusion. This core
   should accept future depth sources without depending on a particular
   renderer or network. A calibrated silhouette visual hull is a separate
   featureless-turntable baseline; it cannot recover hidden concavities and
   must not count its inferred interior as measured surface coverage.
4. **Later: bounded GPU research, when a suitable Kaggle environment is
   available.** Treat [gsplat](https://github.com/nerfstudio-project/gsplat)
   as a CUDA Gaussian rasterizer and fitting reference, not a finished mesh
   pipeline. Treat [GauStudio](https://github.com/GAP-LAB-CUHK-SZ/gaustudio)
   first as a splat-to-mesh experiment: its documented quick start requires
   an existing `cameras.json` and `point_cloud.ply`, and its stated prerequisite
   is an NVIDIA GPU with at least 6 GB VRAM. Do not count these as raw-photo
   reconstructions unless an image-only fitting stage is actually included.
   Compare [NeuS](https://github.com/Totoro97/NeuS) as a separate neural
   implicit-surface fit with image/mask/camera inputs and a mesh export. Check
   camera conventions, object masks, scale normalization and sparse seed inputs
   separately. Use the same two camera lanes where each method supports them;
   otherwise mark the lane unavailable. Measure render quality and extracted
   geometry separately. No GPU job or resource/performance result is claimed.
5. **Optional Apple platform oracle.** On supported Apple hardware, run
   RealityKit [PhotogrammetrySession](https://developer.apple.com/documentation/realitykit/photogrammetrysession)
   on the same permitted image set as a platform-specific end-to-end comparator.
   Record hardware/OS availability, whether pose/mask/scale inputs can actually
   be controlled, and output format. If it cannot consume the frozen oracle
   camera lane, compare only its own estimated-camera result. It is not a
   portable backend decision by itself.

## Source and license gate

Upstream checked 2026-09-26: [COLMAP CPU CLI](https://colmap.github.io/cli.html)
and [CPU FAQ](https://colmap.github.io/faq.html); [OpenMVS source and AGPL
notice](https://github.com/cdcseacave/openMVS), [usage](https://github.com/cdcseacave/openMVS/blob/develop/docs/wiki/Usage.md)
and [optional CUDA setting](https://github.com/cdcseacave/openMVS/blob/develop/docs/architecture.md);
[GauStudio README](https://github.com/GAP-LAB-CUHK-SZ/gaustudio) (MIT except
its rasterizer; the README's future training-pipeline list is not a released
capability); [gsplat README](https://github.com/nerfstudio-project/gsplat)
(Apache-2.0 repository, CUDA/PyTorch); [NeuS README](https://github.com/Totoro97/NeuS)
(MIT repository and image/mask/camera convention); and Apple's
[PhotogrammetrySession API](https://developer.apple.com/documentation/realitykit/photogrammetrysession).
The KIRI announcements linked above are vendor claims and provide neither
independent quality measurements nor the current production source code. These
are upstream claims and interfaces, not tested results in CrispMesh.

Before **any** experiment, inventory the exact component commit, model/checkpoint
and dataset assets, rasterizer, CUDA extension, converter, texture/mesh tools,
transitive packages and output terms. Pin hashes and review each license and
notice independently; a repository's top-level MIT/Apache label does not
clear a different rasterizer or model asset. Before **shipping**, apply
`DEPENDENCIES.md` to the precise linked/bundled artifacts and target platforms:
permissive or MPL-2.0 may proceed through review, GPL/AGPL are excluded, and
LGPL/custom/unknown terms need explicit review. Experimental evidence alone
never approves a production dependency or backend.
