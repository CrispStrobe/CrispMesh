# Dense turntable pipeline: calibrated photos to a closed STL

This directory holds the photo-to-mesh workers of CrispMesh. This README covers
the **all-view dense pipeline**, the route that currently gives the best closed
models from a turntable photo set on an Apple M1 (GPU through PyTorch MPS, or
CPU). The development tree holds further experimental workers (the earlier
plane sweep, the Apple Object Capture and board/SAM session routes). They are
not all published yet and are not described here.

The dense stages start from recovered cameras and one object mask per photo.
`photos_to_inputs.py` produces both from a plain folder of turntable photos
(SAM 2.1 for masks, AliceVision for cameras), and the driver can chain it in
front (see [Inputs](#inputs)).

## What it does

```
inputs  ->  mask repair  ->  silhouette hull  ->  multiscale stereo  ->  TSDF  ->  surface  ->  check
```

| Stage | Module | What happens |
| --- | --- | --- |
| inputs | `dense_all_views_inputs.py` | One camera table for every registered view of an AliceVision scene; raw masks are remapped through the lens undistortion |
| mask repair | `multiscale_stereo.py` | Pixels a single-view segmentation dropped are added back when most views agree in 3D **and** the pixel is object-dark in that photo |
| silhouette hull | `multiscale_stereo.py` | Voxels that project inside (almost) every mask; it bounds all later steps |
| multiscale stereo | `multiscale_stereo.py` | Masked NCC with contrast normalised inside the mask; full depth sweep at the coarsest level, then a narrow band around the smoothed coarser depth; cost aggregation along the surface; a hull-front candidate for thin parts; cross-view depth agreement at every level |
| TSDF | `multiscale_stereo.py` | Depth is averaged into a truncated signed distance volume inside the hull. The silhouette rim band is left to the hull |
| surface | `tsdf_hull_mesh.py` | Unobserved hull voxels fall back to the hull's own signed distance; marching cubes on the float field; largest component; light Taubin smoothing; binary STL |
| check | `mesh_photo_check.py` | Silhouette IoU of the mesh against every mask and a preview sheet |

`scan_evaluate.py` scores a finished STL against an independent scan.
`turntable_rig.py` optionally regularises the cameras with the turntable model.
`dense_events.py`, `engine_server.py` and `export_replay.py` are the interface
for front ends. `dense_pipeline.py` runs all of this as one command. `dense_config.py` holds
every tunable. `stl_compare_render.py` renders several STLs through the same
cameras. `synthetic_scene.py` writes a small analytic test scene.

For detail comparisons, repeat `--crop VIEW X Y WIDTH HEIGHT` on
`stl_compare_render.py`. Coordinates are pixels in that view's input image;
the box is fitted into the panel with the same camera, scale and lighting for
every mesh. Triangles are rasterised again at the closer scale, rather than
enlarging an overview. For example, after a Bunny photo run:

```sh
python -m scripts.turntable_mesh.stl_compare_render \
  --cameras RUN/frontend/inputs/cameras.json --views view_10 \
  --crop view_10 840 420 360 320 \
  --mesh Baseline BASE/mesh/mesh.stl --mesh Candidate RUN/mesh/mesh.stl \
  --output bunny-head.png
```

Meshes and cameras must use the same recovered coordinate frame. These
untextured renders assess geometry; colour and photographed shadows alone do
not establish that a reconstructed feature is present.

Why it is built this way, with the measurements on the 3DLF Dragon behind each
choice, is in [Design notes](#design-notes).

## Quick start

No dataset needed; this runs the whole CLI on an analytic sphere in about 15 s
on CPU:

```sh
cd /path/to/crispmesh
export PYTHONPATH=$PWD
python -m scripts.turntable_mesh.synthetic_scene --output /tmp/sphere
python -m scripts.turntable_mesh.dense_pipeline --inputs /tmp/sphere --output /tmp/sphere-run \
  --device cpu --set sizes=64,128 --set grid=96 --set planes=48 --set neighbours=4 --set best_of=2 \
  --set vote_neighbours=4 --set min_votes=2,2 --set crop_padding=6 --set hull_dilate=1 \
  --set windows=5,7 --set aggregates=1,1
```

A real photo set (defaults are tuned for megapixel photos), either straight from
a folder of turntable photos with a lens calibration, which also runs
segmentation and camera recovery (needs AliceVision and SAM 2.1; see
[`docs/PHOTOS-TO-INPUTS.md`](../../docs/PHOTOS-TO-INPUTS.md)):

```sh
python -m scripts.turntable_mesh.dense_pipeline \
  --photos photos/ --calibration scripts/turntable_mesh/calibrations/3dlf-pro.json \
  --output runs/my-object --device mps
```

or from cameras and masks you already have:

```sh
python -m scripts.turntable_mesh.dense_pipeline \
  --scene final.sfm --prepared undistorted-pngs/ --raw-masks masks/ \
  --output runs/my-object --device mps
```

Result: `runs/my-object/mesh/mesh.stl`, `check/preview.png`, `check/result.json`
and `pipeline.json` (stage timings and status). The output directory must not
exist; nothing is ever overwritten.

### Interpreters

The stages need different libraries, and the local development setup keeps them
in two virtual environments. The driver starts each stage as a separate process:

| Stages | Needs | Selected by |
| --- | --- | --- |
| inputs, surface, check | NumPy, SciPy, scikit-image, OpenCV, Pillow | `--python` or `CRISPMESH_PYTHON` |
| stereo | PyTorch, NumPy, Pillow | `--torch-python` or `CRISPMESH_TORCH_PYTHON` |

Both default to the interpreter running the driver, so one environment with all
of the packages works without any flag. On the development machine:

```sh
export CRISPMESH_PYTHON=$PWD/.local-tools/colmap-sparse/venv/bin/python
export CRISPMESH_TORCH_PYTHON=$PWD/.local-tools/sam21-m1-local-cache-001/venv/bin/python
```

`--device mps` uses the Apple GPU and refuses to run if MPS is unavailable
(`PYTORCH_ENABLE_MPS_FALLBACK=0`); `--device cuda` uses an NVIDIA GPU and
likewise refuses if none is available; `--device cpu` runs the same code on CPU.

### Native stages

The pipeline is being ported to Rust and WebGPU so it can run without Python
(`crates/dense`). The two heavy stages are ported and at parity, and the driver
uses them when given the binary:

```sh
(cd crates/dense && cargo build --release)
python -m scripts.turntable_mesh.dense_pipeline ... --native path/to/crispmesh-dense
```

With `--native`, the stereo stage (mask repair, hull, matching, fusion) and the
surface stage run natively through WebGPU (Metal, Vulkan, DirectX 12) and no
PyTorch is needed. On the 3DLF Bunny on an Apple M1, through the driver:

| | Python stages | Native stages |
| --- | --- | --- |
| Stereo stage | 588 s (idle machine) | 95 s |
| Surface stage | 33 s | 8 s |
| Scanner F1 above the support, 0.5 / 1 / 2% | 0.964 / 0.995 / 1.000 | 0.963 / 0.995 / 1.000 |

Scanner scores of the native stages are within 0.002 of the Python ones on all
four objects; the parity tables are in
[`crates/dense/README.md`](../../crates/dense/README.md). The driver itself, the
input table, the photo check and the photos step (SAM, AliceVision) are still
Python.

### Running stages separately

```sh
python -m scripts.turntable_mesh.dense_all_views_inputs --scene final.sfm --prepared pngs/ --raw-masks masks/ --output run/inputs
python -m scripts.turntable_mesh.multiscale_stereo --inputs run/inputs --output run/stereo --device mps
python -m scripts.turntable_mesh.tsdf_hull_mesh --volume run/stereo/volume.npz --output run/mesh
python -m scripts.turntable_mesh.mesh_photo_check --inputs run/inputs --mesh run/mesh/mesh.stl --output run/check \
  --repaired-masks run/stereo/masks-repaired
```

`multiscale_stereo --reuse-depths run/stereo/depths.npz` skips matching and only
re-fuses, which is the quick way to try fusion settings. Re-meshing an existing
`volume.npz` with other `mesh_*` settings takes about a minute. The driver
deletes `volume.npz` unless `--keep-volume` is given.

## Inputs

Either an existing inputs directory, or the three things it is built from.

**Inputs directory** (`--inputs`):

- `cameras.json`: `{"views": [{name, image, mask, width, height, k, rotation, translation}, …]}`.
  `rotation`/`translation` are world-to-camera. `k = [fx, fy, cx, cy]` in the
  **half-pixel-centre** convention: the ray through pixel `(x, y)` is
  `((x + 0.5 - cx) / fx, (y + 0.5 - cy) / fy, 1)`. Images must be undistorted
  pinhole images. `image`/`mask` may be absolute or relative to the directory.
- `sparse_points.npy`: N×3 photo-reconstructed points; only used to find the
  object's rough location and the scene centre for neighbour selection.
- Masks are 8-bit, object > 127.

**From AliceVision** (`--scene`, `--prepared`, `--raw-masks`): an SfM file with
poses and one shared `radialk3` intrinsic, the PNGs written by AliceVision
`prepareDenseScene` (named `<viewId>.png`), and one 0/255 mask per source photo
with the source photo's file name, in the original distorted frame.

Requirements and assumptions:

- At least `neighbours + 1` views, each with two or more other views between
  `minimum_angle` and `maximum_angle` (3°–40° by default) as seen from the object.
- The object is darker than its backdrop for mask repair to add anything
  (`--set repair_masks=false` turns it off).
- A mask that does not touch the image border is taken to show the whole object.
- World scale is whatever the SfM produced. **The STL has no physical scale.**

## Settings

Every setting can be given three ways, later ones winning: defaults in
`DenseConfig`, a JSON file (`--config`; any earlier `config.json` or
`result.json` works), and `--set key=value` (repeatable; tuples as
`a,b,c`). `python -m scripts.turntable_mesh.dense_pipeline --list-settings`
prints all of them. Lengths are relative (voxels, level pixels, fractions of
depth), so they do not depend on SfM scale. Per-level tuples have one entry per
pyramid level; a shorter tuple repeats its last entry.

| Setting | Default | Meaning |
| --- | --- | --- |
| `sizes` | 256, 512, 1024 | Longest canvas side per pyramid level, capped at native resolution |
| `crop_padding` | 24 | Native pixels kept around each mask box |
| `stretch_percentiles` | 1, 99 | Grey range inside the mask mapped to 0..1 before matching |
| `neighbours`, `best_of` | 6, 3 | Matching neighbours per view; mean of the best N scores is used |
| `minimum_angle`, `maximum_angle` | 3, 40 | Allowed angle between a view and its neighbours, degrees |
| `planes` | 96 | Inverse-depth planes of the full sweep at the coarsest level |
| `windows` | 7, 9, 11 | NCC window per level, pixels |
| `aggregates` | 1, 1.5, 2 | Cost blur sigma per level, pixels; 0 disables |
| `passes` | 1, 2, 1 | Refinement passes per level |
| `band_first`, `band_later` | 8/8/5, 5 | Search half-width in depth steps: first pass of each level, later passes |
| `min_score`, `min_variance`, `window_fill` | 0.55, 1e-4, 0.6 | NCC acceptance, texture gate on normalised grey, minimum valid window fraction |
| `tolerances`, `min_votes`, `vote_neighbours` | 0.006/0.003/0.002, 2/3/3, 10 | Cross-view depth agreement: relative tolerance, required agreeing views, views asked |
| `grid` | 400 | Voxels along the longest side of the object |
| `hull_dilate`, `hull_allowed` | 2, 2 | Mask tolerance in native pixels; views allowed to disagree |
| `repair_masks`, `repair_loose`, `repair_rounds`, `repair_base_margin` | true, 0, 2, 20 | Mask repair on/off; views allowed to disagree (0 = a fifth of the views); rounds; voxels above the support kept out of repair |
| `hull_front`, `hull_front_level`, `hull_front_min_score`, `hull_front_margin`, `hull_front_stride` | true, 1, 0.6, 0.004, 2 | Hull-front depth candidate for thin parts; stride of its ray-march grid |
| `fallback_level` | true | Where the finest level fails its checks, use the previous level's depth |
| `rim_fraction` | 0.55 | Silhouette band excluded from fusion, in matching windows |
| `truncation_voxels`, `behind_voxels`, `behind_weight`, `free_weight` | 3, 12, 0.25, 1 | TSDF truncation; depth and weight of the weak inside vote behind a surface; weight of a free-space vote relative to a surface vote |
| `mesh_smooth`, `mesh_fill_sigmas`, `mesh_final_smooth` | 1.0, 2/4, 0.6 | Field smoothing, extrapolation reach into unobserved hull, final blur; voxels |
| `mesh_minimum_weight`, `mesh_confidence_cap` | 0.5, 4 | Evidence needed to count as observed; views at which confidence saturates |
| `mesh_taubin_cycles` | 5 | Taubin smoothing cycles on the mesh |
| `mesh_flat_base`, `mesh_base_margin` | true, 1 | Cut the solid at the lowest level with measured surface (silhouettes cannot tell a flat base from a cone under it); voxels kept below that level |

Faster, coarser runs: `--set sizes=256,512 --set grid=256`.

## Outputs

| File | Content |
| --- | --- |
| `mesh/mesh.stl` | Binary STL, largest component, outward normals |
| `mesh/result.json` | Triangles, closedness, boundary and non-manifold edge counts, genus, observed and extrapolated hull fractions |
| `stereo/result.json` | Configuration, hull size, mask repair statistics, per-level coverage and timings |
| `stereo/depths.npz` | Final depth per view on the common canvas (`depth_000`, …) |
| `stereo/depth-level-N.png`, `depth-merged.png` | Photo, depth and depth shading for three views |
| `stereo/masks-repaired/` | Masks after multi-view repair |
| `check/result.json`, `check/preview.png` | Silhouette IoU against input and repaired masks; preview sheet |
| `pipeline.json`, `*.log`, `config.json` | Stage commands, exit codes, timings, logs, the resolved configuration |

What the numbers do and do not mean: silhouette IoU is agreement with the
photos' outlines. It cannot see errors along the viewing direction and is not
scanner accuracy. Closedness and genus describe the exported topology, not the
true object. Surface inside the hull that no photo measured (for example the
underside on a turntable) is the silhouette bound, not a measurement.

## Results so far

3DLF turntable sets (73 photos at one elevation, AliceVision global SfM with a
fixed lens, experimental SAM masks), Apple M1 16 GB, PyTorch MPS. Scores are
from `scan_evaluate.py` against the independent structured-light scans:
held-out surface samples, thresholds as a percentage of the scan's
bounding-box diagonal. "Above support" excludes the base, which no photo sees.
The scanner is used for scoring only, never as input.

| Object and route | Stereo time | F1 @0.5% / 1% / 2%, all | F1 @0.5% / 1% / 2%, above support |
| --- | --- | --- | --- |
| Dragon, earlier 24-view plane sweep + Poisson (control 204) | about 2 min | 0.567 / 0.780 / 0.934 | 0.624 / 0.845 / 0.972 |
| Dragon, this pipeline, first version (`mesh-009`) | 749 s | 0.755 / 0.911 / 0.973 | 0.819 / 0.961 / 0.994 |
| Dragon, this pipeline, current defaults | about 7 min | 0.756 / 0.910 / 0.975 | 0.822 / 0.960 / 0.994 |
| Armadillo, no retuning | 426 s | 0.922 / 0.971 / 0.987 | 0.952 / 0.997 / 1.000 |
| Bunny, no retuning | about 7 min | 0.898 / 0.933 / 0.953 | 0.964 / 0.995 / 1.000 |
| Lucy, from plain photos in one command (masks and cameras 7 min, matching 6 min) | 346 s | 0.798 / 0.908 / 0.940 | 0.836 / 0.951 / 0.973 |

All four meshes are closed. Lucy's thin wings and raised arm are partly lost. Surface
extraction adds about 20 s and the photo check about 20 s.

**SAM masks made on MPS before 2026-10-06 came from a partly wrong encoder.**
PyTorch 2.7 on Apple MPS returns wrong values from `max_pool2d` for the strided
query view that SAM 2's Hiera encoder pools at every stage change, so every
encoder block after the first stage was wrong; on the CPU it is right.
`segment.py` now pools a contiguous copy (`contiguous_query_pool`, applied when
the model is loaded; the SAM source is not edited), and its MPS masks then
equal the CPU masks (Bunny: 71 of 73 identical, lowest IoU 0.999996; before the
fix median IoU 0.977, lowest 0.844). Every published score "with SAM masks" in
this repository, including the table above and the dense-stage tables of
`crates/dense/README.md`, was made with the earlier MPS masks.

What the scored experiments on the Dragon showed (each changes one thing):

| Change | Effect on F1 @0.5%, above support | Decision |
| --- | --- | --- |
| Second native-size pass | none (0.819 vs 0.818), +105 s | off by default |
| Two pyramid levels instead of three | 0.791 | keep three |
| Four neighbours instead of six | 0.817 | keep six; four is a reasonable fast setting |
| Rim band 0.3 / 0.9 windows, truncation 2 voxels, 520-voxel grid, less smoothing | within 0.003 | defaults unchanged |
| One or two re-matching passes around the fused surface | 0.825 / 0.823, +200 s each | available (`fused_passes`), off by default |
| Flat support cut | "all" F1 @1% 0.913 to 0.926 | on by default |
| Lower score threshold (0.45) | 0.817 | unchanged |
| Narrower native band (5 steps) and 96 coarse planes | 0.822, faster | new default |
| Turntable model with uniform steps | 0.635, depth coverage collapses | rejected: the recovered step angles are real |
| Turntable model with fitted steps (Armadillo) | 0.890 against 0.936 | not used: the free poses are better than this fit |
| Mask repair tolerating a fifth of the views, two rounds (full reruns) | Bunny 0.955 to 0.964, Armadillo 0.936 to 0.952, Dragon 0.820 to 0.822; restores the Bunny's ears. Whole-surface Dragon F1 @1% 0.924 to 0.910 from contact shadow at the base | new default |
| Repair kept 20 instead of 8 voxels above the support (re-fused, with free-space votes at 0.3) | Dragon whole-surface F1 @1% back to 0.921; above the support unchanged within 0.006 | new default |
| Free-space votes weighted 0.3 or 0.15 (`free_weight`) | Bunny up to 0.970, Dragon down to 0.802, Lucy down to 0.821 | available, off by default |

Fusion and meshing settings barely move the result; what limits the Dragon is
upstream of them (see the camera note below).

Limits, stated plainly:

- **Handedness is unresolved.** Dragon and Armadillo both fit their scans only
  as mirror images, and the photo-derived camera orbits turn in the opposite
  sense to the dataset's depth-derived poses. A fixed-lens perspective SfM
  solution with sub-pixel residuals cannot itself be mirrored, so either the
  dataset's photos are flipped relative to its depth and scan data or those use
  a left-handed frame. Until that is settled, treat an STL's chirality as
  unverified; the evaluator reports which handedness it scored.
- **Camera drift.** The Dragon's recovered orbit has slowly drifting steps
  (about +/-2 degrees around uniform) and camera centres that wander 4% of the
  radius along the axis; an affine correction of the mesh explains part of its
  remaining error (about 3% anisotropy). Replacing the cameras by a fitted
  single-axis turntable model (`turntable_rig.py`) made the scores worse on
  both objects tried, so the cause is not simply free-pose noise and remains
  open; the tool stays available but is not part of the default run.
- **Thin parts and the unseen top.** Horn tips are still short, and surfaces
  no photo sees (the top of the head from a low camera ring, the underside)
  are silhouette bounds.
- Mask repair assumes an object darker than its backdrop.
- Settings were chosen on the Dragon and the Bunny's ears; four objects of one
  dataset (same camera, lens, backdrop and material) have been scored.
- **Camera recovery is not repeatable.** Rerunning AliceVision's global SfM on
  identical features moves the cameras by about 0.7 degrees and 1% of the orbit
  radius with unchanged reprojection error, so the gates cannot see it.
- The cameras come from all 73 photos. This is not a dozen-photo result.
- CUDA (`--device cuda`) was checked once, on a Tesla T4: the unit tests pass
  and the synthetic scene gives the same depths as on CPU (median relative
  difference 9e-8). No real photo set has been run on an NVIDIA GPU.

```sh
python -m scripts.turntable_mesh.scan_evaluate --mesh run/mesh/mesh.stl --reference scan.ply --output run/scan-evaluation
```

or pass `--reference scan.ply` to `dense_pipeline` to score at the end of a run.

## Progress events, live previews and the HTTP engine

Every run writes `events.jsonl` in its output directory: stage starts and ends,
progress fractions with a message, metrics, and an `artifact` event whenever a
file is ready to show. While matching runs, the driver also meshes the
intermediate surfaces coarsely (silhouette hull, hull after mask repair, the
surface after each pyramid level), so a front end can show the model improving.
`--no-live-previews` turns that off; `--preview-step` sets how coarse they are.

```sh
# serve runs to a browser, a phone or a desktop shell
python -m scripts.turntable_mesh.engine_server --runs runs/ --data data/ --port 8765
# make a small bundle a front end can replay without an engine
python -m scripts.turntable_mesh.export_replay --run runs/my-object --output bundles/my-object
```

The event types, artifact kinds, HTTP endpoints and their guarantees are
specified in [`docs/ENGINE-CONTRACT.md`](../../docs/ENGINE-CONTRACT.md).
A cancel is requested by creating a file named `cancel` in the run directory
(the HTTP engine does this for `POST /api/runs/<id>/cancel`).

## Turntable constraint (optional)

```sh
python -m scripts.turntable_mesh.turntable_rig --inputs run/inputs --output run/inputs-rig --steps measured
```

writes a new inputs directory in which every camera is the same camera rotated
about one fitted axis. On the 3DLF sets this made results worse (see the
experiment table), so treat it as a diagnostic, not an improvement. `--steps uniform` also makes the steps between capture
indices equal (median recovered step, or `--step-degrees`); that is right for a
stepper-driven turntable and wrong for a hand-turned one.

## Tests

```sh
export PYTHONPATH=$PWD
$CRISPMESH_TORCH_PYTHON -m unittest scripts.turntable_mesh.test_multiscale_stereo   # config + stereo on an analytic sphere
$CRISPMESH_PYTHON       -m unittest scripts.turntable_mesh.test_tsdf_hull_mesh      # surface extraction on analytic volumes
$CRISPMESH_PYTHON       -m unittest scripts.turntable_mesh.test_scan_evaluate       # similarity fit, platform removal, mirror detection
$CRISPMESH_PYTHON       -m unittest scripts.turntable_mesh.test_turntable_rig scripts.turntable_mesh.test_dense_pipeline
```

Each test file skips the cases whose libraries are missing in that interpreter.

## Design notes

Measured on the Dragon before this pipeline existed:

- **Texture gate.** The old gate was an absolute variance on raw 8-bit grey with
  a fixed 5×5 window. On the dark figure only 50–59% of foreground patches
  passed at 256 px, 26–30% at 512 px and 15–19% at 1024 px, so more resolution
  made depth maps emptier. With grey normalised inside the mask and an 11×11
  window, 98%, 84–92% and 71–84% pass.
- **Neighbours.** 24 of 73 photos gave 15° and 30° neighbours. On a turntable
  the lights are fixed and the object turns, so shading moves across the surface
  between views. All registered views give 5° and 10° neighbours.
- **Masks.** The segmentation dropped whole horns in runs of consecutive views.
  Any strict silhouette intersection then deletes them even though the cameras
  are right. Loosening the hull globally fills real gaps instead, so the repair
  is per pixel and needs both 3D consensus and photo evidence.
- **Thin parts.** A matching window that straddles the silhouette matches the
  apparent contour, which triangulates behind the true surface and carves thin
  parts from both sides. The hull is exact at the rim, so the rim is left to it,
  and thin parts get a depth candidate from the hull's front surface.
- **Surface.** Poisson on oriented points needs normals from depth maps full of
  holes, and re-voxelising its output to a binary grid produced terraces. A
  signed distance volume needs no normals and is meshed from float values.
- **MPS speed.** Every view shares one canvas size per level, because MPS
  recompiles its kernels for each new tensor shape (5–8× slower otherwise), and
  best-of-N neighbour scores use elementwise min/max insertion, because indexed
  `max` and `scatter_` are very slow on MPS.

## Dependencies and licenses

CrispMesh itself is **AGPL-3.0-only** (`LICENSE` in the repository root). Nothing
below is vendored into this repository; the table records what the code imports,
launches or was tested with, so that anyone packaging it can check obligations.
Versions are the ones installed on the development machine on 2026-10-05.
Licenses marked † were not re-read from the pinned source for this list and
should be confirmed before redistribution.

### Imported by the all-view dense pipeline

| Library | Version tested | License | Used for |
| --- | --- | --- | --- |
| Python | 3.11.1 | PSF-2.0 | Runtime |
| NumPy | 1.26.4 | BSD-3-Clause | Arrays everywhere |
| PyTorch | 2.7.0 | BSD-3-Clause | Stereo, hull and TSDF on MPS or CPU |
| Pillow | 11.2.1 / 12.0.0 | MIT-CMU (HPND) | Image reading, resizing, previews |
| SciPy | 1.17.1 | BSD-3-Clause | Distance transform, filters, sparse graphs in surface extraction |
| scikit-image | 0.25.2 | BSD-3-Clause | Marching cubes |
| OpenCV (`opencv-python-headless`) | 4.10.0.84 | Apache-2.0 | Mask undistortion, rasterising, previews |

Installed alongside scikit-image and Torch as their own dependencies (not
imported directly): networkx (BSD-3-Clause), imageio (BSD-2-Clause), tifffile
(BSD-3-Clause), lazy_loader (BSD-3-Clause), packaging (Apache-2.0 OR
BSD-2-Clause), and Torch's bundled components under their own notices.

### Upstream steps this pipeline consumes

| Software | License | Role | How it is used |
| --- | --- | --- | --- |
| AliceVision (fork `CrispStrobe/AliceVision`, branch `crispmesh/m1-sycl`) | MPL-2.0; parts MIT (libmv) | Camera recovery (global SfM), `prepareDenseScene` undistortion; optional native depth/meshing engine | External executables; this pipeline reads its `.sfm` JSON and PNGs |
| AdaptiveCpp (fork `CrispStrobe/AdaptiveCpp`) | BSD-2-Clause | SYCL runtime for AliceVision's Metal depth stages | Linked by the AliceVision build only |
| Apple metal-cpp | Apache-2.0 | Headers for the AdaptiveCpp Metal backend | Build-time only |
| SAM 2.1 (Meta) | Apache-2.0 † (code and checkpoints) | Object masks | Separate segmentation step; masks are files |
| hydra-core, omegaconf, iopath, tqdm | MIT; BSD-3-Clause †; MIT †; MPL-2.0 AND MIT | SAM 2 runtime dependencies | Segmentation step only |
| Eigen, Ceres Solver, Boost, OpenImageIO, nanoflann, LLVM/Clang | MPL-2.0 †; BSD-3-Clause †; BSL-1.0 †; Apache-2.0 †; BSD-2-Clause †; Apache-2.0 with LLVM exception † | AliceVision build dependencies | Build-time and linked by AliceVision only |

### Other engines and tools in this repository's workflows

Not used by the all-view dense pipeline; listed because other workers and
comparisons in the development tree launch them. Some of those workers are not
published in this repository yet.

| Software | License | Role |
| --- | --- | --- |
| MicMac (fork `CrispStrobe/micmac`) | CeCILL-B | Optional comparison engine, CPU only |
| PoissonRecon (binary shipped inside MicMac's `binaire-aux`) | MIT † | Surface from oriented points in the older plane-sweep route |
| COLMAP | BSD-3-Clause † | Optional sparse/dense comparison |
| OpenMVS | AGPL-3.0 † | Optional evaluation backend only; not an approved redistribution dependency |
| OpenMVG | MPL-2.0 † | Optional sparse comparison (see `docs/OPENMVG-LICENSE-CLOSURE.md`) |
| MVE | BSD-3-Clause | Optional comparison engine |
| Apple Object Capture (RealityKit `PhotogrammetrySession`) | Proprietary, part of macOS | Separate Apple reconstruction route |
| LoFTR (via learned matching experiments) | Apache-2.0 † | Camera-recovery experiments |

### Application shell

| Software | Version pinned | License |
| --- | --- | --- |
| Tauri (`tauri`, `@tauri-apps/api`, `@tauri-apps/cli`) | 2.11.x | MIT OR Apache-2.0 † |
| serde, serde_json, libc (Rust) | 1.0.229, 1.0.151, 0.2.189 | MIT OR Apache-2.0 † |
| TypeScript | 7.0.2 | Apache-2.0 † |
| Vite, Vitest | 8.3.1, 5.0.2 | MIT † |

Transitive Rust and npm dependencies are not enumerated here; use
`cargo license` / `npx license-checker` against the lock files before packaging.

### Test data

| Data | License | Note |
| --- | --- | --- |
| 3DLF scan dataset (Bunny, Dragon, Armadillo, …), Mendeley Data `ngvgpsvd8b` v1 | CC BY 4.0 | Photos and independent structured-light scans; not redistributed here |
| YCB object set | See YCB terms † | Not redistributed here |
| ETH3D | CC BY-NC-SA 4.0 | Non-commercial; separately gated; not redistributed here |
| Middlebury stereo | See Middlebury terms † | Not redistributed here |

**Handedness of the 3DLF scans.** Every reconstruction of the four test
objects fits its Revopoint scan only as a mirror image, with the AliceVision,
COLMAP and turntable cameras alike (Dragon, trimmed rms 0.60 mirrored against
1.98 proper). The reconstruction is not what is mirrored. `crispmesh-dense
synthetic --capture --asymmetric` renders 48 photos of a sphere with three
bumps of different sizes on its +x, +y and +z axes, from the rig's camera
convention (x right, y down, z forward, world to camera). From those photos the
default command (threshold masks, turntable cameras) recovers cameras that
match the true ones by a proper similarity (rms 0.0027 against 0.545 for the
best mirrored one, orbit radius 6), and the mesh carried into the true frame
lies on the true object (median distance 0.0017 of the unit radius, 95 % within
0.021). The scan evaluator, given the exact object and its mirror image against
the true surface, scores the first proper and the second mirrored, and our
reconstruction proper (trimmed rms 0.0136 against 0.0140). So the 3DLF scans
are mirror images of the objects as photographed (a left-handed scan export,
or a mirrored frame in the scanner software); the evaluator says so in its
warning and scores the reflected fit, as before.

Engine and data licenses are independent of this repository's AGPL license. A
successful local build or run is not a statement that a combination may be
redistributed.
