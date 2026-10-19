# Surface methods: what we measured and where we go

State on 2026-10-13. This page records one round of comparisons on two captures and the plan that
follows from it. Scans are used for scoring only; every mesh has its own similarity fit to the scan,
so differences of a few hundredths in F1 at 0.5 % are within the fit. Numbers are F1 at 0.5 / 1 / 2 %
of the scan's diagonal and the mean of accuracy and completeness as a fraction of the diagonal.

Captures: YCB power drill and mustard bottle, 300 DSLR photos each from five camera heights, our own
masks and our own recovered cameras (see [OTHER-IMAGE-SETS.md](OTHER-IMAGE-SETS.md)).

## Our own variants, like for like

Same masks (grown, see `mask_grow`), same silhouette hull, same cameras:

| | Drill | mean | Mustard | mean |
| --- | --- | --- | --- | --- |
| Silhouette hull only | 0.697 / 0.871 / 0.991 | 0.0046 | 0.794 / 0.910 / 0.999 | 0.0037 |
| Hull corrected by fused depth (`silhouette_first`, the default for several heights) | 0.818 / 0.921 / 0.998 | 0.0033 | 0.766 / 0.918 / 0.997 | 0.0037 |
| Hull refined against the photos, no depth maps (prototype) | 0.860 / 0.965 / 0.996 | 0.0030 | 0.752 / 0.909 / 0.999 | 0.0038 |

Reading:

- On the textured drill, depth evidence is worth 0.12 in F1 at 0.5 %; refining the hull directly
  against the photos is worth more, and looks as smooth as the hull.
- On the plain mustard bottle nothing beats the silhouette: every photo-based correction is equal
  within the fit or slightly worse. A single-colour surface gives the photos nothing to agree on.
- The silhouette alone needs no matching at all (hull and mask repair about 75 s, mesh 2 s on 300
  DSLR photos), against 8.5 to 10.5 minutes with depth. A mode that stops there does not exist yet.

### Photometric refinement

First a NumPy prototype outside the repository, now a pipeline stage (`photo_refine`, `crispmesh-dense refine`;
see the crate README). Anchor vertices of the hull mesh search
along their normal for the offset at which the photos agree: a 5 x 5 patch on the mesh's tangent
plane is projected into the most frontal view that sees the vertex and into up to five views 8 to
35 degrees away; the score is the mean normalised cross-correlation with the reference patch.
Visibility comes from the mesh. Accepted offsets are spread over the mesh as a smooth field (weighted
diffusion), the mesh moves inward only, and the search repeats with a smaller range (6, 3, 1.5
voxels). About 100 s for the three rounds on 300 half-size photos in NumPy; 3 s per round natively.

Acceptance decides everything. Without any test beyond a score threshold the drill reaches
0.842 / 0.968 / 0.999 and the mustard drops to 0.707 / 0.896 / 0.998 (over-carved sides, broken cap).

| Acceptance | Drill | Mustard |
| --- | --- | --- |
| score threshold only | 0.842 / 0.968 / 0.999 | 0.707 / 0.896 / 0.998 |
| best score at least 0.15 above the median over the offsets (a distinct peak) | 0.860 / 0.965 / 0.996 | 0.752 / 0.909 / 0.999 |
| patches with a grey-level deviation of at least 4 | 0.811 / 0.966 / 0.999 | 0.730 / 0.921 / 0.991 |
| field less one local scatter of the moves | 0.771 / 0.924 / 0.994 | 0.775 / 0.919 / 1.000 |
| all three | 0.716 / 0.880 / 0.991 | 0.798 / 0.913 / 1.000 |

The strict end is safe on the plain object and gives the textured one's gain away; the distinct-peak
test keeps the gain at a small cost on the plain object. Points in between (a peak 0.25 above the median: drill
0.784 / 0.956 / 0.996; the peak test with half the scatter gate: 0.762 / 0.926 / 0.994) lose the drill's gain
without helping the mustard, whose variants all lie within the fit of each other. Rendered, the
distinct-peak result is clean and slimmer than the hull on the drill and is the hull with two or three
small dents on the mustard.

### As a pipeline stage

`photo_refine` (0 never, 1 always, 2 when the silhouette comes first: the default) runs after the surface
stage on `mesh/mesh.stl`, keeps the mesh before it as `mesh-unrefined.stl` and writes `refine.json`.
Distinct-peak acceptance (`photo_refine_peak`, 0.15), 120 000 anchors, six views per anchor, three rounds,
inward only when the silhouette comes first. Photos are cropped to the mesh's projection and reduced so
that the object is at most about 1100 pixels across.

| One command, no settings, 300 photos | Before refinement | After | mesh |
| --- | --- | --- | --- |
| Drill | 0.818 / 0.921 / 0.998, mean 0.0033 | 0.857 / 0.977 / 0.999, mean 0.0026 | closed, genus 1 |
| Mustard | 0.766 / 0.918 / 0.997, mean 0.0037 | 0.737 / 0.913 / 0.995, mean 0.0039 | closed, genus 0 |

From the bare silhouette mesh the stage gives 0.867 / 0.962 / 0.998 (drill) and 0.756 / 0.909 / 0.999
(mustard): the fused depth before it is not needed for this result. The three rounds take 9 s on 300 views;
reading the 300 photos again takes 70 to 90 s (they are not yet shared with the earlier stages). Single-turn
captures are untouched by the default (Bunny and Lucy byte-identical). Not run in a browser.

## Other methods on the same captures

### Gaussian splatting (Brush)

[Brush](https://github.com/ArthurBrussee/brush) (Apache-2.0, Rust and wgpu, commit `1388f74`) run on
its own with default settings on the drill: our cameras and masks in its input format, original colour
photos at half size, 30 000 steps, about an hour on an Apple M1.

- Held-out photos are rendered sharply (30 views, median 25.7 dB inside the mask). That also confirms
  the recovered cameras.
- Its surface is less accurate than its pictures. Points where its rendering turns opaque along each
  ray: 0.685 / 0.846 / 0.946, mean 0.0058. A mesh from those depths (plain voxel fusion, marching
  cubes, no smoothing, our conversion): 0.355 / 0.579 / 0.831, grainy and somewhat inflated, yet with
  relief our meshes lack (chuck rings, housing seams).
- Splat centres lie about 1 cm inside the object, as expected of opaque objects in this method.

Plain splatting optimises appearance; nothing in it asks for one continuous surface.

### Reference runs on a GPU

Two published surface methods on both captures (crops around the object at half size, 544 x 576 and
432 x 704 pixels, all 300 views, our masks and cameras), run on a rented Tesla T4:

- 2D Gaussian splatting with [gsplat](https://github.com/nerfstudio-project/gsplat) (Apache-2.0, v1.5.3):
  our training loop after its reference trainer, 30 000 steps, masks as transparency, initialised from
  our silhouette mesh, every tenth view held out; mesh from its median depths by plain voxel fusion (512
  cubed) and marching cubes.
- NeuS-facto in [hummat/sdfstudio](https://github.com/hummat/sdfstudio) (Apache-2.0, commit `12a3f75`),
  20 000 steps, foreground-mask loss, pure-PyTorch hash grid (no tiny-cuda-nn); mesh by its marching cubes
  at 1024 cubed.

| | Drill | mean | Mustard | mean | time per object |
| --- | --- | --- | --- | --- | --- |
| Ours, no settings (photos at full size) | 0.857 / 0.977 / 0.999 | 0.0026 | 0.737 / 0.913 / 0.995 | 0.0039 | 10 to 12 min, Apple M1 |
| 2D Gaussian splatting | 0.881 / 0.947 / 0.992 | 0.0026 | 0.791 / 0.899 / 0.969 | 0.0041 | 15 to 21 min training, T4 |
| NeuS-facto | 0.892 / 0.964 / 0.992 | 0.0024 | 0.789 / 0.909 / 0.980 | 0.0036 | 118 min training, T4 |

The scores are close; the meshes are not. Rendered side by side, NeuS-facto is in a different class: one
clean surface with the drill's vents, chuck ridges, trigger and housing seams, and a smooth, correctly
shaped bottle. 2D splatting shows the same relief with a rougher skin. Ours has the right overall shape
and none of that relief. F1 at 0.5 % of the diagonal (about 1.4 mm on the drill) does not measure it.
Two weaknesses of the reference methods show as well: the printed label of the bottle comes out as
shallow relief (lettering embossed into the surface, appearance mistaken for shape), and the underside
no camera saw is rough or open, where our flat base is clean. Held-out views of the splats: 25.7 and
24.3 dB inside the mask.

## How established pipelines differ

| | Cameras | Geometry signal | Surface model | Aim |
| --- | --- | --- | --- | --- |
| Meshroom / AliceVision (MPL-2.0) | own structure from motion | patch matching per view (semi-global, needs CUDA) | Delaunay tetrahedra and graph cut | mesh |
| COLMAP, OpenMVS | own | PatchMatch stereo per view | Poisson or graph cut, then mesh refinement | mesh |
| nerfstudio (Apache-2.0) | external | re-rendering loss | none (density or splats) | novel views |
| SDFStudio, NeuS family | external | re-rendering loss | signed distance function | mesh |
| Augenblick (`porto-photogrammetry`, CC BY-NC) | COLMAP or VGGT, then an exact turntable rig | re-rendering loss with surface penalties (SuGaR, 2DGS, PGSR, Gaussian Wrapping) | flattened splats, hull start | mesh |
| CrispMesh | own turntable solver | patch matching per view | silhouette hull with a smoothed correction | mesh |

What they have and we do not: a surface model that is optimised as a whole (signed distance function,
flattened splats) or a global surface extraction over the depth maps (graph cut). Our voxel fusion
averages per-view depths, so their noise becomes lumps; our best results came exactly when a surface
was imposed (hull, smooth correction field, mesh refinement). The Augenblick notes report the same
disagreement between held-out image quality and geometric accuracy that we measured with Brush.

Licences rule several options out as code: the original Gaussian-splatting code and 2DGS, SuGaR and
PGSR are research-only; DUSt3R, MV-DUSt3R and VGGT weights are non-commercial. gsplat, Brush,
SDFStudio and the original NeuS are permissive. None of this is patent clearance.

## Plan

1. **Done: photometric refinement as a pipeline stage** (Rust, CPU), automatic when the silhouette
   comes first. Forced on for single-turn captures it fails the six-object gate (Dragon and Happy Buddha
   lose 0.016 at 0.5 %, Lucy gains 0.012), so it stays off there. Open: sharing the decoded photos with
   the earlier stages, a browser run.
2. **Silhouette-only mode** that skips the main matching: with the refinement after it, the fast path
   to the same result on captures from several heights (about 3 to 4 minutes instead of 10 to 12).
3. **A surface optimised as a whole is the quality target.** NeuS-facto shows what these photos
   contain; nothing in our pipeline reaches that relief. Both kinds (a signed-distance surface optimised by
   re-rendering, and 2D splats) are per-scan optimisers, not models to download; both are wanted, on WebGPU and
   on MLX, natively. First step: the reference model ported to MLX as a prototype
   ([scripts/neural_surface](../scripts/neural_surface/README.md)), started from our silhouette hull with the
   original NeuS sampler limited to a band around the hull, and a smaller network (2 x 64). After the whole
   schedule (20 000 steps, 3.9 h on an M1) the drill scores 0.944 / 0.979 / 0.995 (mean 0.0017) against the
   reference's 0.892 / 0.964 / 0.992 (0.0024), and side by side it shows the same relief. Second step: the
   same model on WebGPU in the engine (`crispmesh-dense surface`, [crate README](../crates/dense/README.md)),
   with hand-written backward kernels checked against the prototype number for number, 0.25 s per step on the
   M1 (MLX 0.6 s): the whole schedule in 95 minutes, 0.932 / 0.976 / 0.995 (mean 0.0019), the same relief.
   It reads a scene straight from an inputs directory, without Python. Mustard bottle from its inputs: 0.795 /
   0.907 / 0.965 (reference 0.789 / 0.909 / 0.980). 2D splats on WebGPU (`crispmesh-dense splats`), checked against a
   PyTorch reference of gsplat's 2DGS: drill 0.877 / 0.944 / 0.992, held-out 25.44 dB (gsplat 0.881 / 0.947 / 0.992,
   25.71 dB), mustard 0.779 / 0.896 / 0.986, 24.51 dB (gsplat 0.791 / 0.899 / 0.969, 24.26 dB); about an hour per
   object on the M1. Both are run stages with quality levels (`--surface` / `--splats fast|standard|best`) and in the
   app's New run form.
4. **Surface extraction** over existing depth (graph cut or similar) as the alternative to voxel
   averaging for single-turn captures.


## Camera recovery and Gaussian alternatives on irregular phone captures (2026-10-18)

Camera estimation and surface fitting remain separate choices. Keep existing algorithms
available when a new algorithm has different tradeoffs; select experimental paths explicitly.
The private manual iPad and Auto iPhone batches currently fail accepted reconstruction quality.
Neither exact turntable degrees nor a replacement capture is a prerequisite for this work.

Our `src/splats/train.rs` optimizes splat parameters using supplied **pipeline-recovered**
cameras; it has no camera-pose optimizer. The existing neural surface also starts from
recovered cameras. Good-looking splat renders cannot establish correct poses or a printable
surface: the appearance model can accommodate geometric error. Earlier Brush and 2DGS
measurements above used recovered cameras and do not demonstrate pose-free recovery.

| Camera path | Purpose and tradeoff | Current evidence |
| --- | --- | --- |
| Existing linear incremental SfM | Calibrated eight-point initialization, linear resection plus nearby-pose refinement; preserves the current baseline | 28/117 iPad, 11/78 iPhone; insufficient coverage |
| Minimal P3P incremental SfM | Three-point calibrated resection with robust sampling, including planar geometry; faster hypotheses, different sample/registration outcomes | Explicit `Resection::P3p`, BSD-3-Clause PoseLib adaptation; 27/117 and 11/78; no shape acceptance |
| Learned camera/geometry initialization | Predict initial poses from several images when repeatable features are scarce, then validate/refine against original photo correspondences | MapAnything Apache prototype, distinct from the noncommercial checkpoint; four-photo calibrated/RGB-only controls still flattened; 336 px full batch memory-limited; 224 px chunked outputs degenerate |
| Dense correspondences and geometric refinement | Test wide-baseline dense matches, cycle consistency and joint camera/point adjustment; more compute and memory | [RoMa](https://github.com/Parskatt/RoMa) and [FlowMap](https://github.com/dcharatan/flowmap) are candidates, not incorporated or verified on these batches |
| Joint camera and Gaussian optimization | Refine approximately initialized poses while fitting a scene, with geometric constraints and an anchored gauge | Not implemented in our trainer; [InstantSplat](https://github.com/NVlabs/InstantSplat) and [NoPoSplat](https://github.com/cvg/NoPoSplat) demonstrate different research approaches, not interchangeable drop-in dependencies |

For the learned path, specifically use
[`facebook/map-anything-apache`](https://huggingface.co/facebook/map-anything-apache),
with code, dependencies and checkpoint revisions recorded separately. A prediction is an
initial hypothesis, not measured depth or an accepted mesh. Object-centric input masking
matters for a rotating object against a stationary background. Preserve crop/resampling
transforms and calibrated principal points; do not optimize the background's stationary pose.

A later capture-assisted path can record synchronized native camera calibration and visual-inertial
tracking for a user walking around a stationary object. Apple's
[ARCamera transform](https://developer.apple.com/documentation/arkit/arcamera/transform) and
[intrinsics](https://developer.apple.com/documentation/arkit/arcamera/intrinsics) provide those
measurements. A turntable requires object motion relative to the camera as well; phone-world
tracking alone is insufficient. These are future capture paths, not repairs for existing JPEGs.

New third-party code and weights incorporated directly must be MIT/Apache/BSD or similarly permissive.
This does not change the project’s existing license.
Review transitive rasterizers and model terms, not only the top-level repository license.
If a GPL-only implementation is considered, use a separate specification/implementation
workflow with explicit subagent contracts; do not label source adaptation as clean-room work.
The present P3P change is an attributed BSD source adaptation.


Private learned-initialization controls use the same four masked iPhone views at 336 pixels.
Supplying the own focal-length/principal-point hypothesis gives median cross-view silhouette
support 0.841; RGB-only gives 0.829. Raw feature depth-reprojection medians remain about
21–55 pixels across pairs, and both rendered foreground clouds remain flattened. Raw matches
can contain outliers, so these are consistency checks, not ground-truth errors. Neither
control is accepted as reconstruction quality.

The 78-view global transformer requires a separate resource strategy on the shared 16 GB M1.
Upstream MPS attention attempts a 90.55 GB buffer. Exact query chunking retains every key/value
and passes small CPU/MPS parity checks, but the 336-pixel full run then exceeds the configured
GPU memory cap in its feed-forward layer. Keep bounded-resolution and overlapping-view
alternatives; upstream `memory_efficient_inference` alone does not bound all intermediates on
this backend. Per-case prediction/failure artifacts must be checked: a wrapper catching an
exception can exit successfully while producing no camera predictions.
Evidence: [learned-camera-real-capture-review.json](../tests/evidence/learned-camera-real-capture-review.json).


With both attention and feed-forward chunking, 224-pixel controls finish for 12 and 78 views
in 7.89 and 64.62 seconds of inference (checkpoint load excluded). **They fail numerical
validity:** every view has a spatially constant point field, constant depth and nonpositive
focal estimates. Silhouette support of a collapsed cloud is meaningless. Small MPS chunked
MLP/attention checks match stock MPS closely; they do not validate the complete mixed-precision
model. Root cause is not isolated. Keep this prototype outside the production provider and
validate against upstream full-precision inference before interpreting method quality.


CPU F32 execution (still using the F16-rounded Apache checkpoint), without AMP or
attention/feed-forward chunking, removes the constant-field symptom at 224 pixels.
Four, twelve and all 78 views now produce varying predictions with positive focal
estimates. This changes several execution choices together and does not isolate a
causal operation. The complete 78-view inference takes 441.1 seconds, excluding
checkpoint load; its median silhouette support is only .650, worst .012. Inspected
clouds contain fragmented/repeated figures. All CPU controls remain rejected for
reconstruction. Apple's system photogrammetry comparison reconstructs the table
from raw images and fails on own foreground crops; it supplies no accepted mesh.

Additional native choices now include five-point initialization, one-to-one
ambiguous resection consensus, splitting falsely joined tracks, and diverse seed
selection. These are additive library options, with existing defaults preserved.
Nine real-batch controls still fail camera coverage and inspected shape. See
[camera path evidence](../tests/evidence/camera-recovery-paths-review.json).


The separate Apache BootsTAPIR tracking control now has a reproducible MPS
sampling-parity workaround and bounded 78-view execution. Dense temporal anchors
plus wider localization tolerances register 39/78; combining independent own
SIFT/LightGlue landmarks registers 49/78. Neither is accepted: temporal-only
median cross-view silhouette support is .716, hybrid .903 (worst .812), and the
inspected point cloud remains poor/flattened. These use different point/view
sets, so support fractions are not paired accuracy scores. No learned tracker
is shipped in the app and no camera default changes. See
[point tracking evidence](../tests/evidence/point-tracking-camera-review.json).

Foreground ALIKED/LightGlue plus native eight-point/P3P/diverse recovery now
registers 76/78 iPhone photos at explicit uncertainty scale 4; strict localization
registers 21/78. Original-resolution SIFT triangulation/joint adjustment improves
its fitting residual from 1.388 to 1.182 px with 76 views, but both corrected surface
controls remain open and visibly inaccurate. An independent COLMAP mapper also
registers 76 with worse silhouette consistency. Camera count and low fitting error
are insufficient. Gaussian/SDF trainers still hold poses fixed; no joint Gaussian
camera optimizer or successful splat-derived STL is claimed. Existing camera,
matching, neural-surface and splat paths remain available. The new learned feature
runner and native recovery example are optional offline tools, with pinned
permissive source/weights and no new app dependency. See
[runner instructions](../scripts/photo_regression/README.md).

The shared Gaussian/neural-surface loader now accepts individual foreground crops
smaller than the common training window. Each such view resamples its valid
window onto the common canvas and scales its horizontal/vertical intrinsics
consistently; it does not pad unknown off-frame pixels as background. The earlier
integer-area path remains exact for views that already fit. This fixes an actual
pre-training failure on the real 76-view iPhone inputs. Three analytic tests cover
legacy pixels/cameras, small valid crops and off-centre ray preservation. Shape
quality still requires a separate training and mesh review.

The bounded iPhone Gaussian control now finishes: 3,000 steps, 320-pixel crop,
192-cell mesh grid, 201 s training / 252 s including evaluation and meshing.
Its 91,384-triangle STL has one component and no boundary/nonmanifold edges.
Inspected grey renders reduce the stereo torso's false grooves, but lose real
relief and leg separation; it remains a coarse alternative. The 20.82 dB median
held-out in-mask PSNR measures appearance, not shape or camera accuracy. Poses
stay fixed. This is not the full 10,000-step fast preset or an equal-budget
comparison. The complete 111-view iPad native pipeline also finishes, but its
closed mesh lacks much of the body/legs (median mask IoU .349) and is rejected.
A stricter camera-tolerance control recovers only 11/117, so simply widening or
shrinking one global gate does not solve the batch. No geometry default changes.

### Bounded learned-depth alternative, 2026-10-19

The offline `windowed_depth.py` control adds DA3-BASE depth to the existing native
fusion path. It does not replace stereo or Gaussian/SDF training. Four-view CPU
windows use fixed own recovered poses/K, physical optical rotations to centre the
object, and robust per-view Z scaling against observed own SfM landmarks. Input
and landmark camera frames must match; a similarly named but different recovery
is refused. Source/checkpoint pins and upstream notices are recorded in the
[source inventory](../tests/evidence/camera-recovery-source-review.json).
No API/evo dependency or noncommercial DA3 variant is used. No model/source is
bundled or added to the app.

On the iPad, an independent shared focal/radial PyCOLMAP fit first makes the body
more complete. Its 110-camera learned-depth control is smoother than native
stereo on identical virtual images/cameras, but loses real relief and leg
separation. Native stereo preserves more irregular detail and false grooves.
Both grey sheets were inspected; neither is a faithful printable reconstruction.
The original 111-camera iPad and 76-camera iPhone controls also complete, with the
same broad tradeoff. Neural depth does not rescue sufficiently inaccurate poses.
The sparse scale holdout is not independent shape validation. Gaussian poses also
remain fixed; no joint camera optimizer has been added.

Inference on 111 views takes 91.4 s with 2.54 GiB peak RSS; this bounds model tensor
memory with four-view windows, not all pipeline bookkeeping. The tested native
DA3 CPU port agrees closely on two inputs, while Metal fails the chosen intrinsic
parity threshold, so the executable recipe uses the official CPU core. Larger
camera models, runtime/app integration, view selection and camera refinement need
further work. See [commands](../scripts/photo_regression/README.md) and
[evidence](../tests/evidence/windowed-learned-depth-review.json). No reconstruction
or geometry default changed.
