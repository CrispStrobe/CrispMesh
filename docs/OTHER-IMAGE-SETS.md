# Other image sets

The seven 3DLF objects share one light-field camera with soft photos, one
light backdrop, one turntable and one matte grey print. This page records how
the native pipeline (`crates/dense`) does on sets that differ in camera, rig,
material and capture path, scored against each set's ground truth. Ground
truth must be used for scoring only in current end-to-end evaluations.
Historical rows below that import dataset masks or poses are **stage diagnostics**,
not valid photo-only adoption evidence. They must not be reused as reconstruction
inputs. Evaluators: `scripts/turntable_mesh/scan_evaluate.py`
(shape-only similarity fit, F1 at 0.5 / 1 / 2 % of the reference diagonal;
`--no-platform` for complete object meshes) and
`scripts/turntable_mesh/dtu_evaluate.py` (the DTU protocol: accuracy and
completeness in millimetres inside the observability mask, above the table
plane, distances under 20 mm).

## Candidate sets

| Set | License (as stated by its authors) | Ground truth | Capture | Size | Our camera providers |
| --- | --- | --- | --- | --- | --- |
| DTU MVS, masks and cameras of the IDR preprocessing | research use; not redistributable | structured-light points, observability masks | robot arm, 49 or 64 views on part of a sphere, 1600 x 1200, rectified | IDR subset 2.1 GB, points 7 GB, masks 6.9 GB | `colmap`; `import` of supplied poses (dense stages only); not `turntable` |
| YCB / BigBIRD Berkeley RGB-D | YCB CC BY 4.0 | Google scanner meshes (16k, 64k) | five cameras over a turntable, 3 degree steps, 1280 x 1024, light booth, glass turntable carrying a chessboard | 0.6 to 1 GB per object | `turntable`, `colmap`; lens with tangential terms |
| Google Scanned Objects | CC BY 4.0 | the meshes themselves | none: photos rendered by `crisp3ds-dense render` | about 1000 objects, 5 to 15 MB each | any, on rendered photos |
| BlendedMVS | CC BY 4.0 | textured meshes, rendered depth | aerial and object orbits, 768 x 576 renders blended with photos | low-resolution set 27.5 GB | `colmap`, `import` |
| OmniObject3D | CC BY-NC-SA 4.0 | scanned meshes | phone video orbits with masks and COLMAP poses; Blender renders | large, behind a login | `colmap`, `import` |
| Tanks and Temples (training) | CC BY 4.0 | laser scans | video around large objects and scenes, no masks | 1 to 5 GB per scene | `colmap`, `import`; masks missing |
| ETH3D | CC BY-NC-SA 4.0 | laser scans | scenes, not single objects (`.local-tools/test-data/eth3d-*`) | small | not object-centric |
| CO3D | CC BY-NC 4.0 | none (COLMAP point clouds) | phone orbits with masks | 1.4 TB in all | no geometric score possible |
| MVImgNet | research use | none | phone orbits | 6.5 M frames | no geometric score possible |

Run here: DTU (four scans), YCB (three objects) and six Google Scanned
Objects rendered as turntable captures (next section). Downloads 16.5 GB on the
VPS (`/mnt/storage/datasets/eval-e/`), of which about 1.6 GB came to the Mac;
the six GSO models are 70 MB.

## Results

Times are wall clock for the whole command on the M1, while the machine was
shared with other jobs (load average 4 to 80), so they are rough.

| Set | Object | Photos | Masks | Cameras | Score | Time | What the sheet shows |
| --- | --- | --- | --- | --- | --- | --- | --- |
| DTU | scan 65, skull | 49 | IDR (import) | DTU poses (dense only) | overall 1.48 mm (acc 1.94, comp 1.02); F1 1 mm 0.69 | 153 s | face and teeth right; the unseen back of the hull stays as a slab (before `mesh_open_unseen`, which gives overall 1.24 mm); ripples on the white dome |
| DTU | scan 65 | 49 | IDR (import) | `colmap` (PyCOLMAP stand-in), ring gates loosened | overall 1.30 mm (acc 1.60, comp 1.00); F1 1 mm 0.67 | 193 s | as above; aligned to DTU by camera centres (residual 0.73 mm) |
| DTU | scan 65 | 17 (every 3rd) | IDR (import) | DTU poses | overall 2.81 mm (acc 3.97, comp 1.66) | 46 s | coarser, more hull left |
| DTU | scan 63, fruit (coloured, glossy) | 49 | IDR (import) | DTU poses | overall 1.19 mm (acc 1.89, comp 0.48); F1 1 mm 0.78 | 106 s | apples, pear and pumpkin clean; base debris |
| DTU | scan 63 | 49 | IDR (import) | `colmap`, ring gates loosened | overall 1.48 mm (acc 2.04, comp 0.92); F1 1 mm 0.57 | 269 s | as above |
| DTU | scan 110, golden rabbit (shiny metal) | 64 | IDR (import) | DTU poses | overall 1.83 mm (acc 2.75, comp 0.91); F1 1 mm 0.60 | 150 s | specular parts noisy |
| DTU | scan 24, house on a table | 49 | IDR (import) | DTU poses | overall 3.97 mm | 193 s | fails: the IDR masks cover 55 to 83 % of the frame, the hull fills the box |
| YCB | cracker box (printed, flat faces) | 60 (6 deg) | dataset (import), undistorted | `turntable` | F1 0.24 / 0.48 / 0.69 | 57 s | box right, a hull cone under it |
| YCB | cracker box | 60 | dataset (import) | `turntable`, `--set support_evidence=false` | F1 0.45 / 0.67 / 0.81 | 52 s | flat base; faces bulge where the print is plain |
| YCB | power drill (glossy red and black) | 60 | dataset (import) | `turntable`, support flattened | F1 0.33 / 0.52 / 0.76 | 56 s | blobby surface on glossy plastic; small in the frame |
| YCB | power drill | 60 | dataset, photos not undistorted, p1 p2 dropped | `turntable` | F1 0.18 / 0.37 / 0.64 (cone kept) | 62 s | about the same as undistorted with the cone (0.19 / 0.38 / 0.64): the tangential term is not what limits it |
| YCB | power drill | 60 | `threshold` | `turntable` | refused by the gates | 9 s | threshold takes the chessboard on the turntable as object |
| YCB | mustard bottle (smooth, nearly symmetric) | 60 | dataset (import) | `turntable`; `colmap` | refused by the gates, both | 7 s, 53 s | too few features on the object (median 241) for either solver |

All runs where the gates passed registered every photo. Where a camera solution
was wrong, the ring and audit gates refused it; no wrong solution reached the
dense stages.

## Google Scanned Objects, rendered

Six GSO models (Copyright 2020 Google LLC, CC BY 4.0, as each model's
`model.config` and `metadata.pbtxt` state; Downs et al., ICRA 2022,
<https://arxiv.org/abs/2204.11918>; downloaded from Gazebo Fuel, owner
GoogleResearch) rendered by `crisp3ds-dense render` as a 3DLF-like capture: 72
views on one ring at 20 degrees elevation (5 degree steps), 1749 x 1155, the
3DLF lens (`calibration/3dlf-pro.json`) with its distortion, 4 samples per
pixel, one light that turns with the camera and casts a hard shadow, 1 px
Gaussian blur, noise sigma 0.01, light disc and backdrop. Then the default
command from the photos (`threshold` masks, `turntable` cameras), scored with
`scan_evaluate --no-platform` against the mesh as placed (`reference.ply`).
The renderer also knows the true cameras: solved camera centres are fitted to
them by a proper similarity and compared (centre error in % of the orbit
radius, orientation error after the fit). Every scored run chose the proper
handedness, and orientation errors of 0.06 to 0.21 degrees under a proper fit
confirm no solution was a mirror image. "Exact masks" replaces the threshold
masks with the renderer's silhouettes (`--masks import:DIR`) to separate the
masks from the rest. Times: M1 shared with other jobs (load 6 to 19), four
threads. Scripts: `scripts/gso_eval/` (`pipeline.sh` per case;
`compare_sheet.py` draws four views of photo, exact mesh and reconstruction
coloured by its distance to the mesh, green 0, yellow 1 %, red 2 % of the
diagonal; `camera_error.py`; `summarize.py`).

| Object | Kind | Masks | Registered | Cameras: centre % / orientation deg | F1 0.5 / 1 / 2 % | Run | What the sheet shows |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Schleich African Black Rhino | dark figurine, thin legs and horns | threshold | 72 / 72 | 0.13 / 0.08 | **0.867 / 0.959 / 0.990** | 161 s | clean; horns, ears and legs right; a little shadow debris between the legs |
| Vans Honey Nut Crunch box | printed box, flat faces | threshold | 72 / 72 | 0.10 / 0.06 | 0.833 / 0.890 / 0.969 | 242 s | faces flat and placed right; grooves along horizontal print lines, dents in the plain yellow band at the top |
| Nintendo Mario figure | colourful toy, white gloves | threshold | 72 / 72 | 0.25 / 0.21 | 0.710 / 0.832 / 0.907 | 160 s | body right; the white gloves and the lit nose are cut away |
| Nintendo Mario figure | | exact | 72 / 72 | 0.19 / 0.15 | 0.830 / 0.947 / 0.979 | 147 s | gloves and face whole; small errors in folds (eyes, straps) |
| Reebok Zig Cooperstown shoe | black and white shoe | threshold | 72 / 72 | 0.22 / 0.15 | 0.431 / 0.645 / 0.814 | 143 s | black upper right; the white logo stripes and white sole parts cut through; ankle opening capped |
| Reebok Zig Cooperstown shoe | | exact | 72 / 72 | 0.22 / 0.13 | 0.597 / 0.735 / 0.843 | 161 s | no holes; the inside of the shoe and the cleated sole, never seen, stay closed or flat |
| ACE coffee mug | patterned mug with a handle | threshold | 72 / 72 | 0.19 / 0.12 | 0.448 / 0.595 / 0.717 | 219 s | outside and handle (with its hole) right; the opening is capped at the rim, the inside not carved |
| Threshold bead cereal bowl | plain white, rotationally symmetric | threshold | refused | | | 31 s | refused by the closure gate (steps add up to 180 degrees); the threshold mask also takes the shadowed disc and drops the lit inside |

Render times were 93 to 161 s per capture of 72 views.

The rhino and the box are prepared as CC BY 4.0 example objects for the app
(`scripts/gso_eval/prepare_dataset.py`: 72 PNG photos each, 230 MB, the
manifest schema of `cstr/3dlf-scan-photos`, GSO attribution); the default
command on that folder gives a mesh byte-identical to the rhino run above.

Where it breaks, with a diagnosis for each:

- **Light parts of an object with `threshold` masks** (Mario's gloves and
  nose, the shoe's white stripes and sole, the white bowl). The threshold
  provider assumes a dark object on a light backdrop; object pixels brighter
  than its level are left out of the mask and the hull carves them away,
  while the hard cast shadow on the disc is taken in (it is carved later,
  since it moves over the disc from photo to photo). With exact masks, Mario
  goes from 0.710 to 0.830 at 0.5 % and the shoe from 0.431 to 0.597: the
  masks cost most of the loss. A mask provider for light or mixed objects
  (colour against the backdrop, or SAM) is the fix; the cameras were right in
  both cases.
- **Concavities the ring never sees into** (the mug's inside, the shoe's
  inside, the shoe's cleated sole). From 20 degrees above, the inner far wall
  of the mug shows only near the rim and the bottom never shows; the result
  is capped at the opening, as the hull is. About half of the mug's reference
  surface is inside, which bounds completeness and so F1. A view from above
  would be needed; this is the capture, not the solver.
- **Print with edges along the ring's epipolar lines** (the box's horizontal
  text lines and panel edges). On one horizontal ring the epipolar lines are
  nearly horizontal, so a horizontal edge gives no disparity and the depth
  wanders along it (grooves up to about 2 % of the diagonal); the plain yellow
  band at the top has no texture at all and dents. A second ring at another
  elevation, or a smoothness prior on plain regions, would address it.
- **Symmetric and plain** (the bowl). A rotationally symmetric white object
  shows no rotation between photos; the solver found half a turn and the
  closure gate refused it, as it should. No wrong camera solution reached the
  dense stages in any of the seven runs.
- **Thin parts** (the rhino's horns, ears and legs, the mug's handle) came
  out right at this resolution when the masks are right; the rhino's thin
  tail is partly lost.

Robustness on the rhino (same render otherwise; default command):

| Change | Registered | Cameras: centre % / orientation deg | F1 0.5 / 1 / 2 % | Accuracy median / p90 % | What the sheet shows |
| --- | --- | --- | --- | --- | --- |
| sharp (blur 0) | 72 / 72 | 0.11 / 0.07 | **0.901 / 0.966 / 0.993** | 0.07 / 0.61 | as the baseline, smoother; skin folds still not resolved |
| baseline: 72 views, blur 1 px | 72 / 72 | 0.13 / 0.08 | 0.867 / 0.959 / 0.990 | 0.11 / 0.71 | clean |
| blur 1.5 px | 72 / 72 | 0.22 / 0.16 | 0.821 / 0.942 / 0.986 | 0.15 / 0.89 | softer surface |
| blur 3 px | 72 / 72 | 0.86 / 0.49 | 0.665 / 0.867 / 0.972 | 0.32 / 1.43 | shape right, surface lumpy; cameras four times worse |
| 36 views (10 degree steps) | 36 / 36 | 0.12 / 0.07 | 0.848 / 0.945 / 0.989 | 0.11 / 0.81 | as the baseline |
| 24 views (15 degree steps) | 24 / 24 | 0.17 / 0.09 | 0.708 / 0.815 / 0.879 | 0.22 / 4.44 | body right; slabs of cast shadow between and beside the legs |
| dark backdrop (grey 0.3) | 72 / 72 | 0.16 / 0.08 | 0.127 / 0.242 / 0.434 | 2.54 / 7.72 | a block: the threshold masks take the backdrop as object (83 % of the frame) |

What this answers:

- **Sharper photos.** Blur moves F1 at 0.5 % by about 0.03 to 0.05 per half
  pixel near 1 px and costs 0.2 at 3 px; within 1 % of the diagonal the loss
  stays small up to 1.5 px. If the 3DLF photos are about 1.5 px soft, a sharp
  camera would gain about 0.08 at 0.5 % on an object like this and little at
  1 % and 2 %. At 3 px the cameras themselves degrade (0.86 % of the radius).
- **Fewer views.** 36 views lose 0.02 at 0.5 %; at 24 views the cameras are
  still right (0.17 %, steps 15.01 degrees, raw turn 359.6 degrees), and the
  loss is the cast shadow: the threshold masks take it in, and 24 views do not
  carve the moving shadow away the way 72 do. With masks that leave the shadow
  out, 24 views would likely hold (not run).
- **Dark backdrop.** The cameras are unaffected; the threshold masks are not
  usable (they assume a light backdrop). The run went through with only the
  warning that the object region touches the frame in all 72 photos; a mask
  covering most of every frame should be refused, as the camera gates refuse a
  wrong solution. Import or SAM masks are needed for dark backdrops.

## What breaks, and what would fix it

- **Support under cameras that look down** (YCB, all objects). With every view
  above the object, the hull continues below the support as a cone as deep as
  `tan(elevation)` times the footprint radius; `support_evidence` takes a cone
  deeper than 15 % of the object's height for a floating object and keeps it.
  `--set support_evidence=false` flattens the base and doubles the cracker
  box's F1 at 0.5 %. Fix (dense stages, `fusion.rs`): compare the depth below
  the support with the cone the camera elevation explains, instead of with a
  fixed share of the height.
- **Captures that do not surround the object** (DTU). The unseen back of the
  hull remained as a slab inside the observability mask, which cost accuracy
  (accuracy 1.6 to 2.8 mm against completeness 0.5 to 1.0 mm). Fixed
  (`mesh_open_unseen`, on): when the views do not go around the object, hull
  surface without measured or extrapolated evidence is left out of the mesh,
  which is then open. Overall, before / after: scan 65 on DTU poses 1.43 /
  1.24 mm, scan 63 1.07 / 1.03 mm; with `--cameras colmap --capture orbit`
  1.34 / 1.30 and 1.39 / 1.31 mm. Closing the unseen part with a membrane
  instead scored between the two (numbers in `crates/dense/README.md`).
- **Non-ring camera paths**: `--capture orbit` keeps the camera audit and
  drops the ring gates (`turntable`, the default, keeps them; the turntable
  provider refuses `orbit`). DTU 65 and 63 with `colmap` and no other flag:
  49 of 49 registered, overall 1.31 and 1.43 mm.
- **Masks that are not the object** (DTU scan 24, where the masks cover most of
  the frame; YCB with `threshold`, which takes the turntable's chessboard).
  Imported masks are needed there.
- **Few features on the object** (YCB mustard). Both solvers fail when
  features are taken inside the mask only. The turntable's chessboard turns
  with the object and would carry the motion, but features over the whole
  photo add the static background, and the order check then rightly refuses
  (neighbours and photos four apart share equally many matches). Tried and
  not kept: a lower feature contrast threshold, and steps fixed to 360 degrees
  over the photos (the turn closed, but the adjustment did not hold the ring).
  Tried next and not kept: features also in a band beside and below the
  mask, keeping off-mask matches only if they move between photos (the
  background stands still), and locating the scene by points inside the
  masks. The bottle then registers (60 of 60, 1 114 features per photo, the
  order check and every gate pass), but the cameras are wrong: the steps
  came out 19 % too large before closing the turn, and the mesh is a hollow
  shell (F1 0.21 / 0.40 / 0.59). The 0.6 to 1.4 turn window for closing a
  turn let that through; it is now 0.75 to 1.15 and refuses such a solution
  (1.19 turns). The axis search (tilts up to 50 degrees) is at its edge for
  cameras looking down this steeply; a wider search, checked against YCB's
  supplied poses, is open.
- **Lens with tangential terms** (YCB, p1 = 0.006). Photos undistorted with the
  full model beforehand gave the same score as dropping p1 and p2, so the
  missing tangential term is not what limits these runs; the calibration format
  (`radialk3`) cannot carry it.
- **Glossy, dark or plain surfaces** (drill, rabbit, skull dome) give noisy
  depth, as expected for photo-consistency matching.


## Photo-only texture follow-up, 2026-10-08

The new texture exporter does not bypass masks or camera gates. A fresh run on
60 mustard photos (already undistorted with the lens calibration), threshold
masks and recovered turntable cameras still fails the order check: median
neighbor matches 25, photos four apart 25. Supplied masks/poses were not used;
no dense mesh or texture was produced. This is a current front-end failure,
not evidence that texture export repairs mustard. The retained mask contact
sheet shows the mask selecting the black undistortion border while excluding
the bottle. This invalid foreground selection must be fixed before attributing
this fresh failure to steep camera angles. Historical runs with dataset masks
remain separate diagnostics; they do not establish current photo-only recovery.

The validation cases serve different purposes: Bunny local facial geometry;
Rhino thin parts and textured rendering; printed boxes seams and ambiguous
horizontal edges; mustard masks plus steep-camera/low-feature recovery; drill
glossy surfaces; Mario/shoe light foreground. DTU needs photo-derived masks
and recovered cameras before it can count as end-to-end adoption evidence.
See [textured meshes](TEXTURED-MESH.md) and [texture evidence](../tests/evidence/photo-texture-review.json).

## Background masks and surface recovery experiments, 2026-10-08

`--masks background` is a new opt-in photo-only mask provider for centred
objects. It estimates peripheral backdrop colour, retains light foreground,
and removes black padding connected to the frame. It does not require a model
or network. It still confuses some cast shadows and foreground colours close
to the backdrop. Threshold remains the default.

Fresh 72-photo GSO shoe and Mario captures registered all views and produced
closed STL meshes and photo-textured GLBs. The regression runner verified that
texturing preserved triangle coordinates and winding byte for byte. Shoe
F1 at 0.5/1/2% of reference diagonal was 0.444/0.643/0.819, versus the retained
threshold run's approximately 0.430/0.645/0.816: a modest change, with rippled
geometry still visible. Mario background masks retained cast-shadow artefacts.
Photo-derived native SAM masks instead gave Mario 0.830/0.947/0.980 and a closed
mesh (genus 14); one inspected mask still lost facial foreground. These are
single runs, not evidence of general robustness. Scanner geometry and rendered
poses/masks were used only for subsequent evaluation, never reconstruction.

Experimental `--turntable-region surface` detects moving features around the
object and jointly fits rotation of a planar support surface to initialize
steep cameras. It uses image correspondences, not known board coordinates or
supplied camera poses. It requires one roughly uniform full turn. Mustard's
photo-derived background masks now select the bottle instead of its black
image border; the joint fit registered 60/60 photos, with a raw turn of 359.4°
and median reprojection error 0.294 px. **The resulting mesh remains badly
wrinkled and open and fails visual quality review.** Camera-fit gates alone
therefore do not establish recovery. This option remains experimental; dense
settings and ordinary camera recovery defaults are unchanged.

Use [the serial regression runner](../scripts/photo_regression/README.md) to
check STL and GLB together. Its PASS is an execution/format check, separate
from shape quality. See [the retained evidence summary](../tests/evidence/photo-robustness-review.json).

A mustard silhouette-only control isolates the damage from stereo evidence:
remeshing the same own-photo hull while ignoring all fused depths produced a
closed coarse bottle, F1 0.637/0.836/0.890 at 0.5/1/2%, compared with
0.181/0.391/0.658 for the joint-camera stereo mesh. Both were scored afterward
with the same reference and evaluation settings. The coarse control preserves
the outline but fills unseen concavities and loses the neck/spout relief. Its
textured GLB preserves its STL triangle coordinates and winding byte for byte.
It is an approximation, not recovered fine geometry.

This control is reproducible from a retained own `stereo/volume.npz` using
existing settings (the large minimum weight exceeds every weight in this run):

```sh
crisp3ds-dense mesh --volume OWN-RUN/stereo/volume.npz --output NEW-COARSE \
  --set mesh_minimum_weight=1000000 --set mesh_hull_overshoot=false \
  --set mesh_open_unseen=false
crisp3ds-dense texture --inputs OWN-RUN/frontend/inputs \
  --mesh NEW-COARSE/mesh.stl --output NEW-TEXTURE
```

This explicitly disables measured stereo evidence and closes unobserved surface;
it must not be presented as a detail-recovery setting or used silently. No
reference geometry is used to construct the coarse mesh.

Drill with its own declared lens calibration (full distortion undistorted before
processing) remains refused: the surface fit had 24.4% support, below its 25%
gate; ordinary object features then failed distinct-landmark support and ring
radius spread (6.60%, limit 5%). The photo-derived masks still miss parts of the
dark chuck in some views. Gates were not relaxed; no accepted drill mesh/GLB
was produced in this follow-up.

For a paired Mario comparison on the same 72 RGB photos, background masks gave
F1 0.542/0.752/0.851; native SAM masks gave 0.830/0.947/0.980. Both recover
cameras independently from those photos. This supports the mask improvement
on this capture, while surface relief remains noisy in the untextured renders.

### Printed-box controls

A fresh GSO cereal-box front end registered 72/72 photos (238.5 s, most spent
matching printed features). Background masks retain the support disk; the
closed stereo mesh also turns printed lines into false geometric grooves.
Photo texture exports successfully, but does not repair these defects.

Native SAM removes the disk but misses some pale top foreground. Preserving
holes caused dark-hole cleanup to refuse one view: 5.28% would be filled,
exceeding its 5% budget. For this solid packaging experiment, the existing
`--no-sam-preserve-holes` policy was replayed on cached own SAM components;
the budget/defaults were not changed. This policy fills enclosed regions and
is inappropriate when actual openings must remain. The resulting masks pass
cleanup; the closed stereo mesh still has false grooves (genus 40).

These controls reuse cameras recovered from the same RGB photos, never dataset
poses. Mask undistortion was checked against native output: all 72 previous
masks reproduce byte for byte. A silhouette-only SAM control still has missing
edges and facets (genus 44). A local prototype combining the two photo-derived
mask estimates gives a closed genus-0 silhouette approximation and about 91.6%
texture coverage. It restores more pale foreground, but visible facets, grey
patches and texture seams remain. This combination is not a shipped provider
or an accepted general box fix. Stronger geometric smoothing softens facets
and corners without resolving the texture defects; no default changed.

The prototype's automatic shape fit narrowly prefers reflection on this
nearly symmetric box. That does not establish physical handedness. The
scanner evaluator warning now states this uncertainty rather than asserting
that the reference must be mirrored. STL/GLB byte preservation and Khronos
format validation pass for these exports; neither check establishes shape
or appearance quality.


### Further controls, 2026-10-08

Disabling mask repair lost useful mustard neck/upper shell and cereal pale
edges. Confidence-weighted fusion did not resolve the wrinkled bottle or
printed false relief; neither change was adopted. A camera holdout reserves
entire match components before fitting: on mustard, 91 held-out observations
of 28 tracks selected using only their fitting observations have median/p95
0.375/1.296 px. The full reserved set is much worse (424 observations,
7.617/128.995 px), exposing bad correspondences. The subset does not establish
local dense accuracy or the accuracy of every camera.

Experimental native SAM background prompts recover more dark drill foreground
in some views. A single candidate fails its positive-point check on photo 18.
Multiple candidates complete 60 masks, but the reviewed sheet contains
checkerboard/background contamination. The rotating-plane camera fit has only
24.8% image support (required 25%); no gate was relaxed and no drill mesh was
accepted. Own matches are retained for correspondence diagnostics.

Coherent texture selection and bounded display-RGB brightness gains give
modest appearance changes on the same meshes. Twenty-four views reduce some
grey source boundaries on cereal, with a 5120-square atlas instead of 4096.
The cereal and mustard meshes still fail the geometric target. Previewing
texture and grey shading in Studio makes this distinction directly inspectable.
See [quality evidence](../tests/evidence/quality-followup-review.json).


### Drill recovery control with automatic cues

A subsequent run enables the existing automatic SAM cues with background
prompts, several candidates and filled mask holes. The common foreground box
and interior/exterior points remove much of the checkerboard contamination.
The same cached RGB feature matches now recover all 60 cameras without
relaxing the rotating-plane support gate (25.42%, required 25%). Median/p95
fitted reprojection error is 0.325/1.213 px; this is not a held-out accuracy
measurement. Remaining masks omit some chuck/battery foreground.

This is the first completed drill mesh/GLB in these controls, not an accepted
quality result. The band baseline is open, with 4,438 boundary edges and
1,006,012 triangles. Cropped comparisons against six source photographs show
pitting and lost molded detail. Independent planes alone leave similar
artifacts (4,313 boundary edges); increasing the minimum neighbor angle to
12 degrees does not make the mesh closed (4,922 boundary edges). Photo texture
renders successfully but conceals some surface errors. No dense or SAM
default changed. The object spans only roughly 200–300 source pixels, so
triangle count does not measure recovered detail.

Rebuilding the independent-plane depths with subpixel fusion also leaves
4,313 boundary edges and nearly the same visible artifacts. All four GLBs
pass Khronos validation with zero errors/warnings and exactly preserve their
respective STL triangle coordinates and winding. These are export checks,
not evidence of recovered shape accuracy.


Drill failure isolation: a fresh baseline reproduces the STL byte for byte.
Raw finest depth estimates already omit broad casing regions and have
irregular slopes. Finest consistent median coverage is 51.4%; coarse fallback
raises it to 71.5%. A diagnostic mesh using only the existing silhouette prior
(no accepted stereo weights, unseen closure enabled, overshoot disabled)
keeps a recognizable coarse shape without the deep pits. It still misses real
surface detail and is not an accepted reconstruction. This points to unreliable
stereo/fusion evidence as a major contributor rather than texture export alone.
The held-out camera subset has 58 foreground observations, median/p95
0.445/1.501 px; the full foreground set is much worse, and background matches
include many outliers. Cycle-confirmed training matches do not improve the
full foreground holdout and were not adopted. Photo-only mask grouping is now
available in the camera diagnostic. See the
[depth failure evidence](../tests/evidence/drill-depth-failure-diagnostic.json).

## YCB power drill from the DSLR cameras, 2026-10-11

The YCB archive `035_power_drill_berkeley_rgb_highres.tgz` holds five Canon DSLR
cameras (N1 to N5, elevations about 8 to 85 degrees) at 120 turntable angles,
4272 x 2848; the RGB-D archive holds the 1280 x 1024 cameras NP1 to NP5 we used
before (NP3, 60 photos). Only the photos and each camera's lens intrinsics were
used; the archives' poses, rig extrinsics and masks stay unopened for
reconstruction, and the Google scan is used only for scoring. Photos every 6
degrees (60 per camera), undistorted with the camera's own calibration; for five
cameras resampled into one common pinhole lens. Our SAM masks and our cameras.

| Same pipeline, production defaults | F1 0.5 / 1 / 2 % (photo-consistency fit) | mean / diag. | F1 0.5 % (mesh ICP fit) | genus |
|---|---|---|---|---|
| NP3, 60 photos, 1280 x 1024 (current) | 0.489 / 0.732 / 0.874 | 0.0093 | 0.523 | 66 |
| N3, 60 photos, half size | 0.603 / 0.757 / 0.852 | 0.0087 | 0.610 | 65 |
| N3, 60 photos, full size | 0.687 / 0.853 / 0.955 | 0.0053 | 0.766 | 26 |
| N1 to N5, 300 photos, full size | 0.724 / 0.855 / 0.937 | 0.0052 | 0.803 | 18 |

Median depth error falls from 0.29 % (NP3) to 0.14 % (N3 full size); depths
more than 1 % off from 13 % to 2 %. Both scan fits rank the runs alike; their
difference (up to 0.08 at 0.5 %) is the alignment uncertainty. Five heights
close the large openings under the motor housing and along the handle (sheet in
the evidence record); their precision at 2 % is a little lower than one turn's,
the newly seen sides being noisier. A finer fusion grid (600, 800) on the same
full-size depths does not help. The battery's underside stands on the glass and
is seen by no camera. Dense stage: 50 s and 2.2 GB for 60 full-size photos,
331 s and 5.4 GB for 300.


## YCB mustard bottle from the DSLR cameras, 2026-10-11

The same preparation as the drill (`006_mustard_bottle`, N1 to N5, 60 photos per
camera, one common pinhole lens from each camera's own intrinsics; supplied
poses, rig extrinsics, masks and depth unused, the Google scan used only for
scoring). The masks are our background-contrast masks: they cut the yellow cap,
the nozzle and parts of labels and lower body in steep views. The yellow sides
have few features, so `--minimum-observations-per-view 15` was given (photos of
the plain side have 16 to 18 points).

All 300 photos registered and every gate passed. Turn 5 (almost straight down)
triangulated no points of its own: it was warned about, and its photos were
registered against turns 1 to 4 and put on the turntable motion (TURNTABLE-SOLVER,
steps 5 and 6). Turn radius spreads were 1.0, 0.6, 1.4, 2.9 and 0.0 %. Production
dense defaults; scores use the scan fitted to each mesh by ICP:

| Run | F1 0.5 / 1 / 2 % | mean / diag. | genus |
|---|---|---|---|
| N3, 60 photos, one turn | 0.725 / 0.889 / 0.950 | 0.0054 | 14 |
| N1 to N5, 300 photos | 0.480 / 0.739 / 0.875 | 0.0089 | 96 |

On this object, adding heights makes the result worse. Both meshes lose the cap
and nozzle (masks), and the five-turn one is wrinkled. Depth rendered from the
scan under our cameras (evaluation only) shows each turn's depth bias: turns 1 to
4 are within 0.2 %, but turn 5 is +1.5 to +1.7 %. Compared with the supplied rig
(evaluation only), our camera 5 is about 4.5 % too far from camera 3. The same
check on the drill gives 0.2 %. Neither a focal correction of the top camera nor
matching across non-neighbouring turns removed the bias. Our best guess is that
the turn is weakly tied to the others: turns 3–4 share a median of 24 matches per
photo pair and turns 4–5 share 36, against 58 and 71 on the drill. Short dense
baselines between nearly top-down views are the other candidate. The cause has
not been established. Subsets with the same cameras: turn 3 alone 0.46 / 0.82 /
0.95, turns 2 to 4 0.49 / 0.82 / 0.96, turns 1 to 4 0.54 / 0.83 / 0.93.
Earlier result on this bottle (NP3, 1280 x 1024): 0.18 / 0.39 / 0.66.

### Silhouette first, 2026-10-12

A silhouette-only mesh of the five turns (`mesh_minimum_weight=1000000`, closed)
already scored 0.683 / 0.827 / 0.974, better than the depth mesh, but its
thickness was 7.5 % too small against its length. A hull cannot be thinner than
the object unless masks cut into it: depth points confirmed from other
directions fell up to 30 % outside the original masks in turns 3 and 4 and up
to 14 % outside the repaired ones. The background-contrast masks leave out the
pale label, and mask repair only adds pixels darker than the backdrop. Because
the depth search is bounded by the hull, the depths were wrong as well, which
adds to the top-turn bias described above.

With silhouette first (`mask_grow`, `cross_check`, `mesh_hull_correction=4` and a
closed mesh; see the crate README), which the default `silhouette_first=2` now
applies by itself to captures from several heights, the same 300 photos and
cameras, one command with no settings ("before" is `silhouette_first=0`):

| Run | F1 0.5 / 1 / 2 % | mean / diag. | mesh |
|---|---|---|---|
| mustard, before | 0.480 / 0.739 / 0.875 | 0.0089 | genus 96 |
| mustard, silhouette first | 0.766 / 0.918 / 0.997 | 0.0037 | closed, genus 0 |
| drill, before | 0.798 / 0.890 / 0.939 | 0.0045 | open, genus 31 |
| drill, silhouette first | 0.818 / 0.921 / 0.998 | 0.0033 | closed, genus 1 |

Each mesh has its own ICP fit of the scan, so differences of a few hundredths
at 0.5 % are within the fit. The rendered meshes are smooth and closed; the
mustard shows its cap and a shallow label panel, the drill no vents or
knurling. The top turn keeps 28 % of its depths in the cross-check, the side
turns 55 to 85 %. Run time 588 s (mustard) and 734 s (drill) against 365 s for
the drill by default. Rejected on the way: depth carving only beyond a fixed
margin, cross-checked depth as the surface itself (lumpy), and growing the
masks after matching without matching again (0.44 at 0.5 %).
