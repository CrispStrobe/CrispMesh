# Depth reconstruction quality audit

2026-10-09. A severe failure is reproducible in the ordinary band matcher with
perfect cameras and synthetic textured photographs. This is a concrete reason
to change matching support geometry, rather than continue smoothing damaged
meshes. It does not establish the cause of every error on real objects.

## Error budget of the drill, 2026-10-10

Own RGB, own SAM masks and own recovered cameras only; the scanner mesh is used
for scoring and as an oracle in separate diagnostics, never as an input of a
shipped run. The reference was aligned to our cameras by maximising
multi-view photo-consistency of its shape (independent of any reconstruction);
the earlier mesh-fitted alignments differed by about 1 % in scale and moved F1
at 0.5 % by up to 0.06, more than most earlier measured improvements.

| Test (evaluation only) | Result | Consequence |
| --- | --- | --- |
| Fusion + mesh from depths rendered from the scan, full coverage | F1 0.5 % ≈ 0.80; every seen part reconstructed, only the unseen underside missing | Fusion and meshing are sound; ≈ 0.8 is the ceiling of this view set |
| Same, only where our depth maps have a value | 0.79 | Coverage is not the bottleneck |
| Same, plus 0.15 % Gaussian noise, white or correlated over 4 px | no change | Noise of our size is harmless if it averages |
| Our depths, pixels with true error > 1 % / 0.5 % / 0.3 % removed | 0.58 / 0.65 / 0.72 (ours 0.54) | The damage is the bulk 0.2–0.5 % error, not outliers |
| Our accepted depths | median error 0.2 % of distance, half smooth (σ 8 px), half pixel noise; fallback pixels no worse | |
| NCC peak with the scan's true shape as window, ±3 views | within 0.3 % for only ~50 % of pixels (±6 views, 6 best: ~60 %) | At 1280×1024 the photos limit per-pixel depth to ≈ 0.2 %; our matcher is near that limit |
| Depth bias by brightness, saturation, slant | uniform +0.09 %, highlights least | Not specular; consistent with the residual reference scale, not a matcher fault |
| Wider sources (`neighbours` 8–12, `minimum_angle` 9) | ±0.01 F1 | Not the lever |
| Fixed truncation 3 → 24 voxels on the same depths | 0.516 → 0.579, mean error −22 % | Averaging was clipped: σ is ≈ 3 voxels, truncation was 3 voxels |

Adopted: `truncation_noise=3` (see the crate README): the truncation follows
the noise measured between the views' own depth maps. A fixed wide truncation
smears the statues (Thai −0.06 at 12 voxels); the measured noise keeps them at
3–5 voxels and gives the drill 8–14. Per-pixel down-weighting by the same noise
estimate did not help (it ranks pixels only weakly).

Not resolved, and not resolvable from these photos by matching changes: the
drill's creases, vents and knurling (≈ 1 mm, ≈ 1.3 px). The next levers are the
input (the YCB capture has 120 frames per ring, five rings and a DSLR set; we
use 60 frames of one 1.3 MP ring) and, at most, multi-view photometric surface
refinement. These tests also rule out a specular depth bias on the real drill
and recovered-pose error as the main factor: the true shape is photo-consistent
under our cameras (windowed NCC ≈ 0.87).

## Independent controls

`crates/dense/tests/depth_geometry_audit.rs` constructs its own analytic scene.
No scanner geometry or dataset-provided cameras, masks or depths enter it.

The coordinate control intersects independently calculated double-precision
world rays with boxes, then checks production projection/unprojection across
48 cameras, unequal focal lengths and off-centre principal points. World-point
error is below 2e-6 scene units and pixel error below .0002 px. This establishes
the tested pinhole convention, not accuracy of recovered real cameras, image
undistortion or every crop/pyramid transform.

Exact analytic depths of a box with a recess and raised feature pass through
production GPU fusion and meshing. The manually constructed outer hull contains
the features but does not encode the recess. Rim removal is zero and artificial
support flattening disabled; other fusion/mesh settings are defaults. At three
independent mesh-ray intersections:

| Surface | Analytic front Y | Exported mesh front Y |
| --- | ---: | ---: |
| Flat face | -.800000 | -.812322 |
| Recess floor | -.600000 | -.604516 |
| Raised feature | -.980000 | -.994825 |

Voxel size is .0208333; each error is below one voxel. The mesh is closed, genus
zero, with 74,772 triangles. Three rendered views were inspected: both features
survive, with softened edges and slight face rippling. This isolates fusion and
meshing with ideal evidence; it does **not** validate their response to noisy,
correlated, missing or occluded stereo depths, or the photo-to-hull front end.

![Exact-depth fusion and mesh control, three views](../tests/evidence/depth-geometry-audit.png)

## A matching failure with perfect cameras

Three parallel cameras observe a textured flat plane at camera depth 2.0.
Reference/source brightness arrays are generated directly from the plane's
texture, with exact integer disparities at truth. Geometry, photographs and
the selected centre pixel's initial depth stay fixed. Only surrounding initial
depths change from 2.0 to 2.12.

| Matching control | Correct surrounding prior | Biased surrounding prior |
| --- | ---: | ---: |
| Ordinary band refinement, cost aggregation sigma 2 | 1.999909 | 1.894572 |
| Ordinary band refinement, cost aggregation disabled | 1.999933 | 1.894488 |
| New coherent band refinement | 1.999933 | 1.999933 |
| Existing independent coherent plane scoring | 2.000000 | 2.000000 |
| Independent plane scoring after the band's result | 1.999909 | 1.998706 |

The band matcher moves a correct centre pixel about **5.27%** away from truth.
This happens before cross-view filtering, fusion or mesh smoothing. The
coherent plane control fixes this controlled failure; it is not evidence of
restored Bunny/drill detail or a six-object adoption result. It uses six
coordinate iterations, stride 1 and fixed fronto-parallel normals appropriate
to this synthetic plane; it is not a claim about the full existing slanted
configuration on real objects.

The mechanism is visible in `shaders/warp.wgsl` and `stereo/matcher.rs`: one
hypothesis adds the same inverse-depth offset to **each patch pixel's own prior
depth**. NCC then scores that deformed patch and attributes its peak to the
centre pixel. Neighbouring prior errors therefore alter the centre's data cost.
For this control the offset that corrects the surrounding prior predicts
`1 / (1/2 + (1/2 - 1/2.12)) = 1.892857` at the already-correct centre, close to
the observed failure. Aggregating cost slices indexed by these offsets adds
further coupling between corrections with different depth baselines.

Warping an entire prior surface can be useful for a coupled surface update, but
its patch score must not be treated as independent evidence for each pixel's
depth. Disabling cost aggregation reproduces the failure, isolating it from
that additional smoothing. The current implementation demonstrably fails that distinction. The
same mechanism can plausibly suppress relief or create false relief; its
contribution to actual Bunny/drill defects still requires a controlled replay.

## Comparison with established pipelines

These are implementation comparisons, not an external benchmark on our images.
Revisions and reproduction values are in
[`depth-geometry-audit.json`](../tests/evidence/depth-geometry-audit.json).

| Stage | Established implementation | Current ordinary Crisp3DS path |
| --- | --- | --- |
| Photometric support | COLMAP derives a patch homography from the centre's candidate depth and normal. | Band refinement warps patch pixels using their individual prior depths plus a common inverse-depth offset. |
| Geometric evidence | COLMAP's geometric stereo evaluates source-depth forward/backward reprojection during optimisation. | Ordinary band matching is photometric; a later filter counts relative depth agreement at rounded projections. |
| Fusion checks | COLMAP checks depth, projection and normals, requires multiple supporting samples and takes median coordinates. OpenMVS dense fusion also tracks depth/confidence/normals, reprojection thresholds and multi-view support. | TSDF receives accepted depths with uniform near-surface votes and explicit free-space/behind-surface votes; ordinary fusion does not receive photometric confidence, source identities or normals. |
| Volume integration | Open3D integrates calibrated depth into a TSDF with a truncation band; its classic implementation also converts Z differences to ray distances. | Calibrated camera-Z differences are integrated with a fixed voxel truncation and an extra weaker behind-surface band, followed by hull-aware field completion and meshing. |

Sources: [COLMAP patch scoring and geometry](https://github.com/colmap/colmap/blob/4ae1aafb16d5eae3d54552b2fc896a4fc9d0f7f4/src/colmap/mvs/patch_match_cuda.cu),
[COLMAP fusion](https://github.com/colmap/colmap/blob/4ae1aafb16d5eae3d54552b2fc896a4fc9d0f7f4/src/colmap/mvs/fusion.cc),
[OpenMVS densification/fusion](https://github.com/cdcseacave/openMVS/blob/a3b6d0d99800a65af6b5f6b20dcc1df4c9e0e0c4/libs/MVS/SceneDensify.cpp),
[Open3D integration](https://github.com/isl-org/Open3D/blob/b6c5e196384ad71e75b6e6f9c5da22d046221f1d/cpp/open3d/pipelines/integration/UniformTSDFVolume.cpp).
No upstream algorithm code was incorporated. OpenMVS is a comparison source,
not a permitted source-code adoption under the owner's permissive-code rule.
Source licenses do not establish patent clearance; prior research restrictions
remain in effect.

## Consequence for further work

An experimental `coherent_band=true` now replaces the band's patch cost with
a coherent candidate surface cost at every finer-level and hull-front refinement.
It searches the full existing inverse-depth band at stride 1, using the centre's
candidate depth and a local-prior normal held fixed during the search. The
coarsest fronto-parallel sweep is unchanged. Offset-indexed cost aggregation is
bypassed because neighbouring centres have different prior depth baselines.
Ordinary default behaviour remains unchanged. Adding only a final slanted
polish leaves earlier matching and filtering damage in the initial surface.
The same perfect-camera control now tests the production coherent band entry:
it keeps depth 1.999933 with either surrounding prior. GPU candidate scores are
checked against independent scalar plane scoring on a synthetic curved surface.
Real photo-only Bunny/drill replays use identical recovered inputs and fixed
evaluation alignments; see [coherent-band evidence](../tests/evidence/coherent-band-review.json).

Then test geometric reprojection and uncertainty-aware fusion as separate
changes. Extra rejection alone can make an already patchy map worse; support
and confidence must remain attached to the evidence. A point-cloud fusion
control can isolate TSDF carving from matching errors. Neither path should
claim quality from a closed mesh, increased coverage or a texture covering it.
Dense defaults still require the existing six-object numerical and visual gate
plus browser execution/memory checks. No defaults change in this audit.

Run the controls serially with a GPU:

```sh
CRISP3DS_GPU_TESTS=1 RAYON_NUM_THREADS=2 cargo test --release \
  --manifest-path crates/dense/Cargo.toml --test depth_geometry_audit \
  -- --nocapture --test-threads=1
```

`CRISP3DS_AUDIT_OUTPUT` optionally retains a newly created mesh directory.
Without it, the test deletes its own mesh output. Camera-ray checks run even
when GPU tests are disabled. The band failure is reported diagnostically; the
test does not assert that the erroneous result is acceptable.


## Real-image correction and limits

Against the same supported-fallback/distinct-mode baseline, strict Bunny head
F1 at .1% of the reference diagonal improves from .612953 to .641016; mean
head surface error falls about 9.9%. Drill F1 at .5/1/2% improves from
.455286/.701277/.929251 to .481374/.720322/.940236, and mean surface error falls
about 4.5%. Reference geometry is consumed only for posthoc evaluation. Local
paired head and whole-object renders were inspected: eyes/nose remain weak and
the drill still has severe pits. This is a measurable correction, **not** an
acceptable completed reconstruction workflow.

Several follow-ups fail on drill: stride-2 patch sampling drops 2% F1 by .0395;
adding the existing finest normal refinement drops it by .0715; a 12-degree
minimum matching angle also worsens accuracy. A separate immutable raw-depth
round-trip filter with a 1-pixel limit slightly lowers all three drill F1 scores.
These changes are not incorporated into the coherent option. In particular,
extra rejection alone does not establish more reliable fusion evidence.

The corrected browser photo pipeline completes all 60 drill photos and exports
an original-RGB textured GLB. WASM high-water memory is 1.24 GiB; this is not
browser/GPU process memory. The mesh is still open and about 31.6% of its area
has no selected texture. Texturing preserves the geometry and does not repair
its defects. Timing is one serial run, not a controlled speed comparison.


The combined option fails the default gate on Lucy: .5% above-support F1 drops
.00327045 versus ordinary defaults (allowed loss .003). Against the same
supported-fallback/distinct-mode baseline the drop is .00232646, isolating the
additional matcher contribution. The adoption run stopped there; Armadillo,
Dragon, Thai statue and Happy Buddha were not run for this candidate. No default
or successful-six-object claim follows from Bunny improvement.

Fusion-weight replays reuse the exact corrected drill depth file. Reducing
free-space weight to .25 improves strict .5% F1 (.481374 to .492182) but reduces
2% F1 (.940236 to .932007) and increases mean error. Removing the additional
behind-surface votes worsens every threshold, with or without the reduced
free-space weight. Inspected meshes remain badly pitted. These are negative
controls, not an uncertainty-aware fusion implementation or adopted changes.


A final private check changes existing normal refinement to stride 1, keeping
all other settings and inputs fixed. Strict drill F1 increases slightly to
.489211, but 2% F1 falls to .888842 from .940236, and mean error grows. Denser
sampling therefore does not remove the combination's broad loss. This one-line
experiment is reverted; the production coherent band continues using stride 1
while the separate legacy normal-refinement option is unchanged.

## Independent drill controls, 2026-10-09

Actual independent implementations now test separate stages on the same drill
photos. They do **not** establish an acceptable repair or an end-to-end external
SOTA result. Supplied poses, depth and masks remain excluded; the independent
mesh is consumed only for posthoc scoring with the existing frozen alignment.
Full records are in [drill-independent-controls.json](../tests/evidence/drill-independent-controls.json).

OpenCV CPU SGBM rectifies original RGB using our recovered pinhole cameras,
checks left/right disparity and reprojects camera Z into the native crop.
It produces 777,343 valid pixels versus 773,806 for corrected native stereo.
On identical common pixel locations, mean point-to-reference surface error is
.008820 versus .006676 of the reference diagonal. More valid depth therefore
does not establish better depth. Passing these maps through native fusion
worsens the drill. The generic diagnostic runner reproduces all 60 privately
retained control depth arrays exactly, without reading native depth as search
data. It is two-view regularized stereo, with different appearance processing
and source selection, not a multi-view stereo quality benchmark.

Independent PyCOLMAP 3.11.1 sparse SfM with fixed intrinsics and our foreground
masks fragments into four models (largest 21 views). Adding a manually selected
rotating-table interior from RGB connects all 60 views, with 2,391 sparse points
and .594 px mean training reprojection error. Its own cameras and sparse points
also produce a badly pitted mesh through native dense reconstruction. Its
positive-scale gauge alignment uses only native and COLMAP camera centres,
never the evaluation scanner or supplied poses. Changed sparse points also
change the hull, support and depth bounds, so this is not a pure pose ablation.
Neither training reprojection nor registration count clears local dense accuracy.

Open3D 0.19.0 CPU TSDF receives identical corrected native depths, cropped
intrinsics and the same five-pixel square mask-rim erosion. An independent
analytic-plane control checks camera-Z integration (maximum Z error .0000221).
It bypasses our hull prior, support flattening, field extrapolation and smoothing.
At the native voxel/truncation it creates 5,325 disconnected components; keeping
only the largest drops substantial upper-body evidence. The full output is
therefore also reviewed and scored. Increasing truncation from 3 to 12 voxels
still fragments it (2,074 components). This control retains the input depths'
errors; it does not establish an error in TSDF integration alone.

An existing permissively licensed PoissonRecon tool receives measured positions
from corrected native depths and normals estimated from smoothed inverse depth.
It improves continuity visually, but does not restore the drill's structures.
Its strict F1 rises while mean error and broader F1 worsen. It infers missing
surface and remains rejected as a repair.

| Same frozen evaluation alignment | F1 .5% | F1 1% | F1 2% | Mean symmetric error / diagonal |
| --- | ---: | ---: | ---: | ---: |
| Corrected native control | .4814 | .7203 | .9402 | .007919 |
| Independent SGBM, native fusion | .3089 | .5057 | .7080 | .014706 |
| Independent COLMAP cameras/sparse, native dense stages | .4858 | .7605 | .9327 | .007683 |
| Open3D TSDF, all components | .5210 | .7454 | .8768 | .009615 |
| Poisson from corrected depths | .5309 | .7619 | .8800 | .011960 |

The direct support audit finds **39.25%** of 100,000 area-sampled native mesh
points have no depth within the fusion truncation. This is support attribution,
not geometric accuracy: samples are after extrapolation/smoothing, and omitting
rim erosion makes counted observations an upper bound. The existing TSDF weight
also includes distant free-space and behind-surface vote mass. A new report
field `weight_kind` makes that interpretation explicit. Neither the TSDF
`observed_fraction` nor meshing's `observed_hull_fraction` is a measured surface
coverage fraction.

A private GPU experiment preserves the signed-distance average but replaces
confidence with counts of near-surface views. Its analytic plane test passes;
real drill controls worsen. Closing its gaps with the silhouette prior also
worsens them. The shader/setting changes are reverted, not shipped as another
experimental knob. These failures rule out that tested correction, not the need
for uncertainty-aware fusion.

The next substantive work must attach photometric ambiguity, source-view support
and geometric uncertainty to each depth estimate, then test consistent surface
fitting before hull completion. In particular, a broad or correlated match must
not gain measured-detail credit merely from a high score or several similar
views. Per-view geometric checks must participate in candidate selection, rather
than only delete the final pixels. Camera sensitivity and genuinely withheld
photo evidence still need validation; this follow-up does not provide a full
photo-heldout experiment. Completing or texturing holes is not detail recovery.

The independent libraries are tools only. Their licenses do not establish
patent clearance; no upstream algorithm source or production dependency is
incorporated. The Open3D binary lives in an isolated diagnostic installation.
See the [OpenCV SGBM API](https://docs.opencv.org/4.x/d2/d85/classcv_1_1StereoSGBM.html),
[PyCOLMAP API](https://colmap.github.io/pycolmap/pycolmap.html),
[Open3D TSDF workflow](https://www.open3d.org/docs/release/tutorial/pipelines/rgbd_integration.html)
and [Open3D MIT license at v0.19.0](https://raw.githubusercontent.com/isl-org/Open3D/v0.19.0/LICENSE).
