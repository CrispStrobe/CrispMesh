# Photo-only regression checks

## Automatic real-capture route

The app/native run default is now a silhouette surface; Gaussian is optional
with the Capture preset (3,000 steps, crop 320, mesh 192). Stereo is retained
with `--base-surface stereo`. Existing stereo experiments below keep their
original settings.

On a Mac, `capture_pipeline.py` accepts original JPEG/PNG photos and performs
EXIF orientation, the app's Vision foreground detection and automatic
mask-spike recovery, ALIKED/LightGlue matches, foreground-edge filtering, PyCOLMAP radial camera
recovery, sparse support auditing and coverage-aware view selection. No
per-batch pixel crop, supplied pose or precise motor angle is needed. Mixed
image dimensions/calibration groups are currently refused. Recognition and
camera recovery can still fail; this is not a guarantee for arbitrary objects.

Upright originals remain byte-exact. The native stage crops each object view
automatically with 15% padding. `--shared-crop` retains a separate common-frame
crop path. Automatic foreground includes SAM 2.1 refinement with a validated
coarse-mask fallback, matching the preparation of the earlier good Gaussian
control. `--no-refine-masks` retains the faster native-foreground path.
`--masks` accepts the app's saved foreground masks and preserves their contours;
`--refine-imported-masks` also refines those explicitly supplied masks.
The refined route requires `sam_source` and `sam_checkpoint` in the tools
configuration. The host selects refined foreground when these are installed;
native foreground and imported preview masks remain separate choices.

Supply a host-only JSON with absolute paths for `native`, `foreground`, `python_torch`,
`python_geometry`, `sam_source`, `sam_checkpoint`, `lightglue_source`,
`extractor` and `matcher`. Model runners verify their existing source/weight
pins. Sources and weights are not bundled or downloaded by this command.
Each photo is detected independently before trying the previous search area,
so arbitrary changes of viewpoint do not silently crop away thin parts. Isolated
oversized selections are retried inside trusted neighbouring regions. Build
`foreground` from the same app implementation on macOS 14 or later:

```sh
cargo build --release --locked --manifest-path tools/foreground-capture/Cargo.toml
```

```sh
python scripts/photo_regression/capture_pipeline.py \
  --photos /data/originals --tools /host/tools.json --output /data/runs/new
# Add --gaussian for the default 3,000-step refinement.
# Add --data-root /data to retain a linked scene for later refinement.
```

Automatic focal initialization is 0.9 times the larger original dimension,
zero radial distortion, subsequently optimized from correspondences. A
measured calibration or `--fov DEG` can provide the initial lens instead.
Up to 8% of recovered views are excluded using sparse support, while guarding
against direction coverage loss. The final mesh is also checked against all
recovered views. This differs from manually forcing a fixed view count.

The default camera search starts with a hybrid graph: up to four neighbours
proposed by masked RootSIFT overlap, plus ALIKED/LightGlue retrieval and temporal
links with context padding 0.30 shifted inside the image. Image order proposes
links, never equal motor angles. If a model registers fewer than 80% of accepted
photos or median sparse-point mask support is below 85%, the runner tries the
retained tight path (padding 0.15, six retrieval neighbours and two temporal
neighbours on each side), then the wide path (padding 0.30, twelve and four).
`--camera-search tight|wide|hybrid` changes this search order. Every path must pass
the registration and sparse-support gates. Sparse-acceptable candidates also
build the actual default silhouette and check it against every recovered view.
Median silhouette IoU below 0.8 rejects the camera path; at least 0.85 ends the
search early. Otherwise the highest-scoring eligible candidate is selected after
trying the retained alternatives. This catches camera fits whose sparse points
look plausible while the silhouette loses thin parts. The score measures agreement
with the estimated masks, not independent shape accuracy.

Views with too few surviving sparse observations are removed before
export using the native minimum (20, or eight for fewer than 25 accepted photos),
including any cascading loss of tracks. This consumes the same 8% view-selection
budget; native input validation is unchanged. Candidates, removed views and the
selected path remain in `camera-path.json` and the recovery report.
The main result must reach median mask IoU of 0.8 across recovered views.
If optional Gaussian misses the check, the actual base is checked and kept as
the main result when it passes. Both meshes remain selectable, with an
English/German explanation. Neither passing means failure with diagnostics
retained. The threshold is unchanged. Own-mask agreement is not true shape
accuracy; a smoother optional result can still be useful.
These checks detect some failures; they do not establish geometric accuracy.
Content-checked feature and pair caches can be shared with `--cache-root`.

The raw route accepts 8–1,536 photos. Stage deadlines scale with photo count
(at least 30 minutes per stage), while cancellation, log and disk budgets
remain active. This is an input limit, not a claim that every capture will
register or fit in every device's memory. The 119-original iPad verification
exceeds the former 96-photo limit.

For app use, run `python -m scripts.turntable_mesh.engine_server --runs
/data/runs --data /data --capture-tools /host/tools.json` and connect the app
to that host. The phone is an HTTP client for this route, not a Python host.
Follow the server's existing network/token configuration when sharing it.
The host serializes heavy jobs and guards an 8.8 GiB free-space reserve.

`--resume` checks the input/recipe fingerprint, command and every cached
output byte. Incomplete stages are rebuilt; previous successful stages remain.
The operator-only `--adopt-completed` migrates a locally generated predecessor
without receipts and explicitly records this in the report. Such a run is
not an uninterrupted cold benchmark. `pipeline.json` separates resumed elapsed
time from the sum of successful stage times. Reusing a recovered scene avoids
repeating masks, matching and camera recovery when adding Gaussian later. Raw and cached runs use the same
full-view surface selector. New exports retain content-checked validation
inputs as well as selected training inputs, so excluded views are still checked
during cached refinement. The prepared_capture.py runner does this without loading the
learned preprocessing models.

Run cases serially through masks, recovered turntable cameras, stereo and mesh.
This historical runner explicitly selects `--base-surface stereo` so changes
to the app default do not silently change the meaning of existing regressions.
With `--texture`, export GLB from each resulting STL and verify every triangle
coordinate and winding byte. Reference meshes, supplied poses, dataset masks
and dataset depths must not be reconstruction inputs. The caller must verify
photo provenance; the runner cannot establish it from a directory name.

```sh
python scripts/photo_regression/run.py --manifest cases.json \
  --binary /path/to/crispmesh-dense --output /path/to/new-results --texture
```

Example local manifest (paths resolve beside the manifest):

```json
{"cases": [
  {"id": "light-object", "photos": "capture/photos", "calibration": "capture/lens.json",
   "masks": "background", "require_closed": true,
   "minimum_textured_area_fraction": 0.6,
   "attribution": "The capture's required attribution"}
]}
```

`masks` may be `threshold`, experimental `background`, or native `sam` (requires
an appropriate build/model/runtime). Optional `photo_options` contains CLI
arguments such as `--turntable-region=surface`; data/provider overrides and
imported match caches are refused. Surface recovery is experimental, requires
one roughly uniform full turn with moving planar support features, and has
not yet produced acceptable mustard geometry.

Results retain logs, reports, masks and meshes. The free-space threshold defaults
to 9 GiB; reconstruction stops if crossed. An open mesh fails unless the case
explicitly sets `require_closed: false`, for example for unobserved undersides.
A PASS means execution, closure (when requested), texture coverage and geometry
preservation passed. It does **not** mean the shape is accurate. Render and
review untextured meshes and score against an independent reference separately.
Texture coverage also does not establish colour accuracy.

Run the geometry-check test with:

```sh
python -m unittest discover -s scripts/photo_regression -p 'test_*.py'
```

## Camera observations reserved before recovery

`camera_holdout.py partition --matches photo-matches.json --output split`
removes entire connected match components, including ambiguous ones. Recover
cameras using `split/training-matches.json`, then run:

```sh
python scripts/photo_regression/camera_holdout.py evaluate \
  --matches photo-matches.json --reserved split/reserved-tracks.json \
  --cameras recovered/cameras.json --calibration lens.json --output scores
```

The evaluator triangulates alternating observations and scores the others.
It reports all reserved observations and a subset selected using only the
fitting observations (at least three, each below 1 px). Bad correspondences
remain visible in the full report. This is a camera diagnostic, not a dense
accuracy gate. Matches must use the original calibrated photo coordinates;
calibration is the engine's `radialk3` model. Do not fit with the original cache.

Add `--foreground-masks` to `evaluate` to report errors separately inside and
outside the recovered cameras' photo-derived masks. The masks must match each
camera's image dimensions; normalized feature coordinates are mapped using
that camera's intrinsics. These groups do not alter triangulation, track
selection or the overall scores. Report group counts: a small number of good
object observations cannot establish accuracy everywhere, and incorrect masks
can misclassify observations. Points beyond the mask bounds form a separate
group rather than being counted as background.

When comparing recovered cameras, add
`--selection-cameras baseline/cameras.json` to **both** evaluations. This freezes
verified-track eligibility using the baseline's fitting observations, without
looking at the held-out observations. Candidate points are still triangulated
with candidate cameras. Use only your own recovered baseline cameras; supplied
dataset poses must not select the subset. The ordered view names must match.
Both baseline and candidate cameras must have been recovered with the same
partitioned training cache, excluding all reserved components. A baseline
recovered with the original full cache would leak the held-out observations.

Report `fit_verified_selected_observations` and
`fit_verified_invalid_observations` alongside the errors. A candidate with
behind-camera or untriangulatable selected observations must not gain credit
by dropping those observations from its percentiles. Fixed-subset comparisons
require zero invalid selected observations. The full reserved-track and
foreground reports remain necessary: a better selected median with a worse
tail or full foreground error does not establish better cameras for stereo.

## Local relief through the stages

`detail_profiles.py --spec regions.json --output new-folder` compares raw,
filtered and merged depth maps and meshes with an independently aligned scanner.
Scanner geometry is read only by this evaluation script. It never writes
reconstruction inputs. NumPy/SciPy are required; plots also need Matplotlib
(or pass `--no-plots`). A specification contains:

```json
{"cameras":"own-inputs/cameras.json", "evaluation_reference_stl":"aligned-scan.stl",
 "regions":[{"name":"eye","view":"view_10","crop":[1048,505,90,90]}],
 "depths":{"raw":{"archive":"stereo/depths-pass-0-raw.npz",
                     "metadata":"stereo/depths-stage-cameras.json"}},
 "meshes":{"final":"mesh/mesh.stl"}}
```

Select and inspect boxes on the source photographs first. Crops are original
photo coordinates, mapped through each stage's actual intrinsics. Reports
include coverage, ray-depth error, inverse-depth relief after plane detrending,
and Gaussian high-pass relief at 2/4/8 px. Each stage has its own common-ray
statistics and a second comparison restricted to the same rays across **all**
stages. Filtered depths equal raw values where retained, so on the fixed
intersection they can have identical errors while coverage falls. Report both.
Alignment error, crop boundaries and missing data can affect these measures;
a gain or correlation is not proof that an eye or nose has been recovered.

## Independent stereo and surface evidence controls

`stereo_control.py` runs the installed OpenCV CPU SGBM implementation on pairs
of original lens-undistorted RGB photos, using your own masks, recovered cameras
and sparse points. It does not read native stereo depth as search data. OpenCV
is an optional diagnostic dependency; it is not added to the app. Inputs must
have a common pinhole canvas and native-resolution crop metadata from
`CRISPMESH_STAGE_DEPTHS=1`. Keep original RGB paths in the control's
`cameras.json`, rather than contrast-preprocessed matching images.

```sh
python scripts/photo_regression/stereo_control.py \
  --inputs own-original-rgb-inputs \
  --stage-cameras native-run/stereo/depths-stage-cameras.json \
  --output new-independent-stereo --pair-gap 4 --threads 2
```

The output `depths.npz` can be replayed through native fusion with
`run --reuse-depths new-independent-stereo/depths.npz`. Compare both its depth
points and its mesh on frozen evaluation alignment. This is two-view SGBM,
not an external COLMAP MVS or SOTA benchmark; differences in preprocessing,
regularization and selected source views remain. The original control uses
one source per reference, a five-pixel block, two-sided disparity verification
at one pixel, sparse-derived disparity bounds and nearest Z-buffer reprojection.
An independently generated plane with an eight-pixel disparity tests the
conversion from rectified disparity to original camera Z.

`surface_support.py` samples a mesh by triangle area and counts its own depth
observations within the actual fusion truncation. It reads no scanner. Pass
truncation and behind-band distances in scene units, not voxels:

```sh
python scripts/photo_regression/surface_support.py \
  --mesh native-run/mesh/mesh.stl --cameras own-inputs/cameras.json \
  --stage-cameras native-run/stereo/depths-stage-cameras.json \
  --depths native-run/stereo/depths.npz \
  --truncation 0.003 --behind-distance 0.012 --output support.json
```

Use the run's actual distances. Samples are after mesh smoothing/extrapolation,
not exact fused voxel centres; mask-rim erosion is omitted, so observations
are an upper bound on production support. A supporting depth is not proof of
correct geometry. Far free-space votes, weak behind-surface votes and direct
near-surface observations are counted separately. No automatic acceptance
threshold or default change follows from this diagnostic.

Optional PyTorch/MPS sampling diagnostic (no model download): run
`python scripts/photo_regression/torch_grid_parity.py --layout contiguous --output <report.json>`.
Compare `--layout strided` in a **separate fresh process**. The strided control
fails on the tested PyTorch 2.7.1/M1 environment, while contiguous inputs agree
with CPU within 0.000001. Mixing layouts in one process can affect the result;
this check does not isolate the backend implementation cause or validate a full
model. PyTorch is an optional research environment dependency, not added to the
app. See `tests/evidence/point-tracking-camera-review.json` for full-model parity
and the rejected real-photo geometry controls.

Object-only camera consistency can also be checked without reference geometry:

```sh
python scripts/photo_regression/sparse_silhouettes.py --model <colmap-model> \
  --masks <own-foreground-masks> --output <report.json> \
  --minimum-median-support .95
```

The optional caller-selected threshold returns exit status 2 on rejection; without it
this is a report only. Every recovered landmark stays in the denominator,
including behind-camera and off-frame points. Mask dimensions must match each
camera. `--pixel-scale 4` is only for a deliberately quarter-scaled diagnostic
model; it does not alter bearings or resample source images. By default a
deterministic sample of at most 20,000 object landmarks bounds projection memory.
Do not include background/support landmarks. Low support identifies inconsistency;
high support alone does not prove good poses or shape. Different landmark/view
sets are not paired accuracy scores. Four analytic tests check raster coordinates,
scale, excluded/behind/off-frame support, nonfinite inputs and dimension validation.

## Optional foreground learned features and general cameras

`learned_matches.py` runs CPU F32 ALIKED-n16rot (BSD-3-Clause) and LightGlue
(Apache-2.0) from separately supplied pinned code/weights. It downloads nothing.
The source/weight revisions and license evidence are recorded in
[`camera-recovery-source-review.json`](../../tests/evidence/camera-recovery-source-review.json).
Prepare a small external `lightglue/` package containing only the pinned
`aliked.py`, `lightglue.py`, `utils.py`, and an empty `__init__.py`; the script
checks their hashes and both checkpoint hashes. Retain their upstream notices.
No SuperPoint or other feature model is needed. Torch, Torchvision, Kornia,
NumPy and Pillow belong to the optional research environment, not the app.

The template supplies only `names` and `lens` from your **own** photo match-cache
schema (`crispmesh_turntable_matches_v1`). Calibration must match these oriented
photos, including any previous crop. Lens principal points and output keypoints
use integer pixel centres. Supply masks as `<photo-name>.png`, in photo dimensions.
Foreground crops keep aspect ratio and use white padding outside the frame;
resize inversion preserves pixel centres. Mask/RGB/model hashes fingerprint
per-photo feature caches. Completed pair matches are cached separately, with
index/one-to-one validation and atomic writes. A pooled-descriptor graph chooses six neighbours plus
two nearby image indices in each direction, without assuming equal angular
steps. Similarity construction remains quadratic; costly learned matching has a
bounded candidate budget. Optional `--reuse-overlap` also selects up to four
neighbours from template matches, whose preprocessing cost must be disclosed.

```sh
python scripts/photo_regression/learned_matches.py \
  --source /path/to/pinned-selected-package \
  --extractor /path/to/aliked-n16rot.pth \
  --matcher /path/to/aliked_lightglue.pth \
  --template own-photo-matches.json --photos own/photos --masks own/masks \
  --output learned-features

cargo run --manifest-path crates/dense/Cargo.toml --release \
  --example recover_cameras -- \
  --matches learned-features/matches.json --output new-own-colmap-model \
  --resection p3p --seeds diverse --seed-budget 8 --measurement-error-scale 4
```

To prepare dense inputs directly, append `--photos own/photos --masks own/masks
--inputs new-native-inputs` to the recovery command. This uses the production
scene writer for consistent radial-lens undistortion and per-view foreground
cropping. Masks may use the source filename or `<source-filename>.png`.
The model and inputs paths must be fresh. This is the preferred route for the
nonzero radial lens in the iPad experiment; its RGB/masks must stay in the same
original distorted frame as its declared lens.

The recovery example exposes the independent library choices (`--help` lists
all). Defaults retain the library's earlier algorithms/tolerances. The multiplier
4 is an explicit localization experiment, not inferred camera calibration or
recommended universal setting. The exported model retains the actual lens and
original pixels. Check coverage, sparse silhouette consistency and untextured
surface renders separately. All these paths are experimental offline tools;
they are not selectable app providers or a successful shape-quality guarantee.

Export your own undistorted COLMAP result with `colmap_inputs.py`:

```sh
python scripts/photo_regression/colmap_inputs.py --model new-own-colmap-model \
  --photos own/photos --masks own/masks --output new-native-inputs \
  --normalize-world
```

This adapter requires optional PyCOLMAP and refuses nonzero distortion; undistort
RGB/masks consistently first if needed. Both COLMAP and engine input K use
**half-centred raster coordinates**, so this conversion does **not** subtract
`.5`; it subtracts only integer crop offsets. A diagnostic scaled model can
explicitly use `--model-pixel-scale`. The production `SceneWriter` already handles
its different integer-centred `Lens` correctly. Optional world normalization is
one consistent similarity of cameras/points and preserves projections. Physical
scale remains unknown. Source hashes/crops and coordinate convention are retained
in `provenance.json`. Never use supplied dataset poses/masks as pipeline inputs.

### Independent camera mapping and optional learned depth

`colmap_recover.py` adds an independent BSD PyCOLMAP mapper for the same own
foreground match cache. Keep native recovery for comparison. The current adapter
requires equal focal lengths and `k2=k3=0` (SIMPLE_RADIAL); it refuses other lenses
instead of discarding coefficients. Run with the existing PyCOLMAP environment:

```sh
python -m scripts.photo_regression.colmap_recover \
  --matches learned-features/matches.json --photos own/photos \
  --calibration radial --output own-colmap-recovery
```

`fixed` (default), `focal`, and `radial` are separate calibration controls.
Principal point stays fixed. `report.json` names the largest connected model;
a tiny two-view model with a lower fitting residual is not preferred. Selection
is not quality acceptance. Import that model with the ordinary `photos` stage,
its `own-estimated-lens.json`, the original photos and own masks. Keep the
resulting `photo-map.json` alongside the inputs. Estimates are not measured lens
calibration. Pixel centres are converted once, in each direction.

`windowed_depth.py` is a separate offline CPU path using the **Apache DA3-BASE**
checkpoint and pinned official core. No model/source is bundled or downloaded by
the runner. Obtain the exact revisions/hashes in
`tests/evidence/camera-recovery-source-review.json`, retaining upstream notices.
Do not substitute noncommercial DA3 variants. It calls the core directly; the
upstream API wrapper and its `evo` dependency are not used. Optional dependencies:
PyTorch, safetensors, OmegaConf, addict, einops, OpenCV, SciPy, NumPy and Pillow.

```sh
python scripts/photo_regression/windowed_depth.py infer \
  --inputs own-frontend/inputs --photo-map own-frontend/photo-map.json \
  --sparse-model own-colmap-recovery/model-1 \
  --source /path/to/pinned/Depth-Anything-3 \
  --checkpoint /path/to/DA3-BASE/model.safetensors \
  --config /path/to/DA3-BASE/config.json --output new-windowed-depth

crispmesh-dense stereo --inputs new-windowed-depth/inputs \
  --output new-levels --only levels --set sizes=168,336 \
  --set native_level=false --set silhouette_first=0

python scripts/photo_regression/windowed_depth.py reframe \
  --inputs new-windowed-depth/inputs --depths new-windowed-depth/depths \
  --level-report new-levels/result.json --output new-reframed-depths.npz

crispmesh-dense run --inputs new-windowed-depth/inputs --output new-learned-mesh \
  --reuse-depths new-reframed-depths.npz --set sizes=168,336 \
  --set native_level=false --set silhouette_first=0
```

Use the selected model from your report, not necessarily `model-1`. Omit
`--photo-map` only when camera sources already retain original COLMAP names.
Mapping is explicit and one-to-one; no positional matching. Every output must be
fresh. Finish inference before reframing; partial depth files are not a complete
run. There is no inference-resume option. For controlled comparisons use the
same geometry settings at preflight and execution (including mask repair/growth,
`photo_refine`, and `hull_allowed`), and run native stereo on those same virtual
inputs without `--reuse-depths`.

Physical virtual-camera rotations centre the object without falsely centring an
off-axis crop. Four-view windows follow recovered directions, without prescribed
angle steps; camera poses remain fixed. Each view's depth scale is robustly
anchored to its own observed SfM tracks. The 20% track holdout tests scale fitting
only, not independent camera/shape accuracy. Missing scale support produces an
explicit zero plane/report, never invented metric scale. Native fusion needs the
actual working-crop dimensions/K from preflight; source-sized depth arrays are
not interchangeable. Reframing preserves camera rays and Z depth.

CPU F32, two workers and a 4 GiB RSS guard are deliberate. The 111-view test used
four-view windows, 2.54 GiB peak RSS and 91.4 s for model inference/scale export;
this is a desktop execution benchmark, not an app/device or 1536-view benchmark.
Learned depth can smooth false engraving and also erase real hair, hands and leg
separation. Inspect grey geometry and mask consistency; keep stereo/Gaussian/
learned results as alternatives. No automatic best-quality claim or default switch.


### Large native camera fits

The experimental native recovery example now exposes
`--bundle-solver auto|dense|iterative|off`. `auto` retains dense Schur adjustment
through 128 recovered cameras and uses matrix-free, block-preconditioned conjugate
gradients above that count. Previously, adjustment was silently skipped above
128. All camera and landmark variables remain in the same robust least-squares
objective; this is not an independent per-window camera fit. The iterative path
stores per-observation/point blocks and camera vectors, without a camera-count
square matrix. A solve is bounded to 256 PCG iterations, checked against the true
linear residual, and accepted only when the full robust objective decreases.
Failed linear solves retry with more LM damping.

Use `--bundle-solver iterative` to compare on smaller real batches. `dense` is
explicitly capped at 128 input views; `off` provides a no-adjustment control.
Reports identify the selected solver, whether a final update was accepted and
its before/after robust cost on the fixed pre-pruning tracks. These are fitting
diagnostics, not camera/shape accuracy. Lens intrinsics stay fixed. The native
recovery example remains an offline experimental path, not an app provider.
This removes an optimization skip; it does not establish large real-capture
quality, physical scale or a 1536-view runtime/memory bound.

### Mask boundaries and inconsistent views

`refine_masks.py` adds a conservative SAM2.1-tiny boundary path for **own** coarse
masks, including Vision masks exported by the capture app. It places foreground
cues across a grid using both coarse interior distance and image contrast.
Candidates must satisfy those cues and retain at least 85% of the coarse
foreground, with at most 15% spill and 2% cleanup. Predicted score ranks only
eligible candidates: a high-scoring torso-only mask cannot win by score alone.
If none passes, the original coarse mask is retained and explicitly reported.
These gates constrain changes to an uncertain prior; they do not prove accuracy.
Tiny islands/pinholes are cleaned, while larger holes and open channels remain.
The ordinary native import stage still applies its existing dark-hole policy.

```sh
python scripts/photo_regression/refine_masks.py \
  --photos own/photos --masks own/coarse-masks \
  --source /path/to/pinned/sam2-source \
  --checkpoint /path/to/sam2.1_hiera_tiny.pt --output new-refined-masks
```

The exact external Apache source files/checkpoint are pinned in
`tests/evidence/sam21-refinement-source.json`. No source/model is bundled or
fetched. The original archive did not retain its git revision; the inventory
therefore pins file content, not a guessed commit. Run in a fresh process.
Image-only inference excludes the installed combined-license `tqdm` implementation:
an original import shim implements no progress functionality and raises if any
video/progress operation is attempted. No restricted implementation is rewritten.
CPU F32, two workers and an RSS guard are deliberate; MPS is not selected.
Use `--limit` for a small diagnostic prefix. Outputs are fresh, not resumable;
`progress.json` distinguishes completed masks and explicit coarse fallbacks.
Inspect source photos beside refined masks before using them for geometry.

`select_views.py` is an independent optional path when some camera/mask views
are inconsistent. Rank by an own `sparse_silhouettes.py` report, keeping camera
poses, original ordering, world coordinates and the **full-batch** sparse bounds.
An angular coverage guard refuses removing a distinct direction without a nearby
retained view; it does not require uniform steps or a complete turn.

```sh
python scripts/photo_regression/select_views.py \
  --inputs own-frontend/inputs --photo-map own-frontend/photo-map.json \
  --support-report own-sparse-silhouette-report.json \
  --keep 100 --output new-selected-inputs
```

Tie the report to the same own recovery/photos; name matches/hashes alone cannot
establish that relationship. `--link-inputs` can hardlink immutable prepared data
on the same filesystem to avoid copies. This is reconstruction with selected
views using a rig estimated from the full batch, not reconstruction from 100
photos alone. Compare meshes against **all** original views, including excluded
ones (`crispmesh-dense check --inputs original-inputs --mesh result.stl
--output new-all-view-check --check-views 110` for 110 views). Coverage thresholds
and count are explicit controls, not automatic best-quality guarantees. The
original all-view path remains available.

Before native import, validate the candidate masks against its unchanged cleanup
policy and image evidence for new interior cuts:

```sh
python scripts/photo_regression/validate_masks.py \
  --photos own/photos --coarse-masks own/coarse-masks \
  --refined-masks new-refined-masks --output new-validated-masks
```

This stage keeps the coarse mask when the proposed refinement exceeds the native
2% dark-hole fill budget, adds unsupported interior boundaries, or is empty.
The native rule concerns enclosed 4-connected background, at most 2048 pixels,
whose photo pixels are all at most luminance 128; preflight uses a conservative
2-level luminance margin for decoder differences. It does not raise the native
budget. Boundary evidence is a local image-gradient check, not proof of a real
hole; true low-contrast openings may revert to the prior. Inspect the explicit
fallback reasons. The raw candidates remain available as a separate path.

Refined masks can also remove cached feature correspondences from background or
unstable silhouette edges **before** camera recovery:

```sh
python scripts/photo_regression/filter_matches.py \
  --matches learned-features/matches.json --masks new-validated-masks \
  --margin-pixels 3 --output new-foreground-matches.json
```

Keypoint arrays/indices, calibration and original pixel coordinates are retained;
only edges are removed. The extractor/candidate graph is reused, so this is not
a new cold-start matching benchmark. Run camera recovery again on this cache;
changing only the final hull masks cannot remove background tracks from an
already recovered camera model. Compare the all-view and selected-view paths.
The iPad control retains 64,114 of 71,965 matches and 465 of 470 pairs, then registers
116/117 with the independent focal/radial mapper. Registration does not establish
correct cameras; sparse silhouette consistency still fails the requested .95 gate.

Native imported-camera + imported-mask runs now preserve upright JPEG bytes
while staging, rather than expanding every source into a full-resolution PNG.
Canonical capture aliases are internal IDs and native readers decode by content;
this shortcut is restricted to the entirely native import route. Identity
contrast also copies the original bytes. Rotated photos still become upright
PNGs, external-tool routes keep their existing PNG stages, and final prepared
scene images remain PNG. JPEG decoded-pixel/codec/orientation tests pass. No
photometric matching or geometry setting changes as part of this I/O fix.

## Optional focus selection control

`focus_views.py` creates a separate subset from an own recovered scene. It
normalizes object size to 512 pixels, measures contrast-normalized RGB gradient
energy inside the mask, and compares views within 20 degrees. A photo below
0.65 of its nearby 75th percentile is a candidate; removing it must retain at
least eight views and stay within 20 degrees of a retained direction. Texture,
noise and lighting can affect this estimate. Originals are never deleted.

```sh
python -m scripts.photo_regression.focus_views \
  --inputs /data/own-recovered-inputs --output /data/focus-selected-inputs
```

This is an opt-in diagnostic, outside the app defaults. The iPad control
removed 23 of 106 training photos, including the owner's out-of-focus example.
With the same own cameras and hull, Gaussian 3,000 produced no clear visual
improvement: all-116-view median mask IoU was 0.86779 before and 0.86703 after;
some surfaces became more faceted. These scores measure agreement with estimated
masks, not shape accuracy. Removing training images also changes the training
view sequence and held-out split, so held-out PSNR is not directly comparable.
This control does not test whether excluding blur before feature matching would
improve camera recovery. It does not justify automatic deletion in the app.
