# Photo-only regression checks

Run cases serially through masks, recovered turntable cameras, stereo and mesh.
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
