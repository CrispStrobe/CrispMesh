# Board-free cracker-box visual-hull control — frozen proposal

Status: **runner implemented and synthetic tests only**. No real voxel carving,
mesh export, reference scoring, or large copy has run. This is one development
object-geometry experiment, not an App Store backend selection or a mustard
checkerboard continuation.

## Why this one experiment

The fresh image-only 60/60 cracker-box producer has a separately reviewed
named-camera agreement of 1.894% center RMS over orbit radius and 2.896° p95
orientation error. Its full-resolution OpenMVS dense cloud is already too broad
before meshing; refinement only changes fixed-gauge F@1% of the independent
Google reference from 31.79% (rough) to 30.92% (refined). The fitted refined
F@1% is 42.23%, still poor. Thus another mesh/refinement setting is less
informative than asking whether the **same image-derived cameras and masks**
can geometrically constrain the object at all. A strict visual hull is a
single, deterministic silhouette-support upper envelope that uses no depth or
photometric stereo. If it is also broad/incorrect, those inputs are
insufficient for this shape; if it is materially better, the dense-depth path
is implicated. The masks are only coarse photo-derived foreground support,
not certified silhouettes, so a hull failure cannot distinguish mask error
from camera error. Bunny's 73-photo results are less decisive for this test:
its scanner includes a large support disk and whole-object registration has
unresolved orientation/coverage disagreement.

## Exact sealed inputs and coordinates

Read, without copying or modifying, the completed
`/Volumes/backups/code/crispmesh-data/turntable-fresh-openmvs-005` producer
`result.json` (SHA-256
`dafc6ccb2a90f0c5a45748291571455237acd8ae08da01dd80adc10222a3625e`),
its `masks/report.json` (SHA-256
`dc5ff13e84f66b810709a85aef41b6459abb6b9f78f99996e632862572923fef`),
and exactly the 60 name-paired undistorted PINHOLE cameras, RGBs, and native
binary masks listed there. The undistorted `dense/sparse` COLMAP model hashes
are `cameras.bin` `c746ffa33ade1b725399771b99915904a53d2a134c5738fc94a1ebaf70d84ee4`,
`images.bin` `cb6a663a0d5ffae69a5f1b2335408ea00b447c8aa83ff7028759843e61a45ae8`,
and `points3D.bin` `8b8dd2d156ee517b8dfb8a2dab00c7925e6ba18cdf06e4db3f7b5d5f14f18693`.
Every native mask and undistorted RGB must rehash to its **per-name** SHA-256
in the sealed mask report, with exact 60-file inventory, dimensions and 0/255
labels. The result/report/model and all 60 mask/image hashes are checked
before and after. No Berkeley calibration/poses, depth maps, Google mesh,
reference transform, held-out view, or reconstructed dense cloud enters hull
construction or bounds.

For each world voxel center, use the unchanged COLMAP `cam_from_world`:
`x_cam = R @ x_world + t`; require a proper right-handed R and positive depth.
For PINHOLE `(fx,fy,cx,cy)`, project `u=fx*x/z+cx`, `v=fy*y/z+cy` in COLMAP
pixel coordinates, then sample the native mask at `floor(u),floor(v)` when
`0<=u<width, 0<=v<height`. A view outside its image is unobserved, not a
negative silhouette. The voxel survives only if **every** in-frame view has
mask value 255 and at least 48 of the 60 views observe it. No erosion,
dilation, soft vote, depth consistency, or candidate-dependent mask choice.
The pure synthetic guard [module](../scripts/classical_backend/visual_hull_core.py)
and [tests](../scripts/classical_backend/test_visual_hull_core.py) lock pixel
edges, cheirality, handedness, exact binary masks, all-view conjunction and
minimum-view behavior; they do not touch the real capture.

## Frozen grid, output and abstention

Use only the finite 005 undistorted sparse model's 3D points (require at least
1,000) for bounds: coordinatewise 1st/99th percentiles, center at their
midpoint, cube side `1.5 * max(percentile_span_xyz)`. This gives 25% padding
on both sides of the largest central span, without scanner or dense-cloud
geometry. Evaluate exactly a 160×160×160 cell-center grid in lexicographic
XYZ order, chunks of at most 65,536. A nonempty hull touching any outer grid
cell is **truncated and abstains**; do not expand/recenter after seeing it.
All-empty is also an abstention. Convert occupied cells to a neutral triangle
mesh by emitting only exposed axis-aligned cube faces, deduplicating lattice
corners, and splitting each quad along the same fixed diagonal; no smoothing,
hole filling, decimation, texture, or scanner-driven cleanup. Seal occupancy
count, bounds, camera/mask hashes and bare mesh SHA-256 **before** reference
access. Produce candidate-only untextured XY/XZ/YZ preview with fixed views,
not a reference-overlay selected from its score.
For an empty or boundary-touching grid, write a structured `abstained` receipt
with exact occupied count, boundary-touch boolean, reason, grid bounds and
input hashes; emit no mesh or preview. A missing/malformed view or changed
input hash fails in the supervisor's full read-only validation before the
output directory is created; the worker independently revalidates before
carving to guard against an intervening source change.

Proposed fresh external output:
`/Volumes/backups/code/crispmesh-data/cracker-visual-hull-001`.
No existing artifact is overwritten; no large source copy or download. Cap
grid memory/RSS at 2 GiB, worker time at 300 seconds, two CPU threads, total
new output at 256 MiB, triangle count at 500,000, report at 2 MiB and preview
at 16 MiB. Require both internal and external disks to retain >10 GiB plus
the output allowance before/through/after work. A supervisor kills the worker
at 330 seconds. Exceeding any cap or a failed hash/geometry check is a
recorded abstention, not permission to resize the grid or rerun variants.
The [bounded producer](../scripts/classical_backend/cracker_visual_hull.py)
contains no scanner/reference import. Its child is supervised by the existing
resource monitor, which checks sampled child RSS, disk floors, output bytes,
log bytes and deadline. The pure-core and binary-PLY/neutral-preview synthetic
tests run without sealed inputs. Proposed command after explicit approval:

```sh
PYTHONDONTWRITEBYTECODE=1 .local-tools/colmap-sparse/venv/bin/python -m scripts.classical_backend.cracker_visual_hull --output /Volumes/backups/code/crispmesh-data/cracker-visual-hull-001
```

## Separate scanner-only score, unchanged golden

Only after the candidate mesh and neutral preview are sealed, read the same
independent Google scanner PLY
`.local-tools/test-data/ycb-cracker-box/reference/google_64k_geometry_f64.ply`
(SHA-256 `6e0187aa961aef4fa21dfc753023a4be7e82924e6398dc0875a2b0d609363ed4`)
and frozen 005 score report (SHA-256
`165b8d5169e8bdb917f008864b01c434f4e03455b6ffe688d8ae8a824b2847c9`).
Primary comparison uses its existing **camera-transported 006 shared-gauge**
matrix, unchanged for the hull and 005 rough/refined baselines. Secondary
reference-fitted proper Sim(3) uses the same 1,024 surface samples and seed
2026 as the earlier score, reported separately; it cannot rescue a failed
primary comparison. Use the unchanged whole-scanner 0.5/1/2% reference-bbox
diagonal thresholds, 2,048 area-weighted samples per direction, seeds
2027/2028, and reference self-control. Report precision, recall and F
separately, directional p95 distance, topology, mesh AABB extent, occupied
voxel volume, and fixed-gauge volume ratio to the watertight scanner. At each
threshold, `1-precision` is **extra surface fraction** and `1-recall` is
**unsupported scanner surface fraction**; they are not physical mass. Report
also the fraction of occupied voxel centers outside the reference AABB as a
clearly labeled lower-bound extra-volume proxy, not an overlap/IoU claim.
Do not crop the scanner or candidate and do not select a score-favorable fit.

A coherent improvement would require both fixed-gauge precision and recall
at 1% to rise by at least five percentage points over the sealed 005 rough
mesh, without boundary truncation, with a plausible neutral box preview;
fitted-only F improvement, precision gained by collapsing coverage, or a
large volume mismatch is not a win. A poor/degenerate hull falsifies the
sufficiency of this **mask+camera** support but cannot by itself assign fault
to segmentation versus pose. This development-object comparison is not
physical metric accuracy, KIRI parity, or production acceptance.
