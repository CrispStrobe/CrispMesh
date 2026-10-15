# Mustard common-crop dense control: partial, not accepted

The approved `mustard-sparse-masked-dense-003` control preserved the structurally reviewed
48-view, image-estimated fixed-initial/exhaustive sparse poses and repaired
tracks. It used only the 48 accepted TRAIN coarse-mask foreground boxes to
choose one common half-open crop `[493,329,715,631]`, padded 32 original
pixels, yielding 222×302 images. Every RGB crop was a lossless PNG of its
decoded original JPEG pixels; native masks used the same crop without
resampling. One copied shared camera shifted its principal point by the crop
origin; all copied 2D keypoints and image names were rewritten, while 48 poses,
939 XYZ points, 4,734 track observations and the `points3D.bin` bytes stayed unchanged.
All 4,734 tracked forward projections/rays were checked after binary
serialization (maximum translation difference 1.14e-13 pixel; ray difference
zero). The original model, photos and masks remained unchanged.

The [new runner](../scripts/classical_backend/crop_masked_dense.py) SHA-256 is
`69115524d01414996fe66cdcab26b470749db8d92cf293ad8cddef3c713517f8`.
The six [focused tests](../scripts/classical_backend/test_crop_masked_dense.py)
passed, including synthetic PyCOLMAP serialization and the local sealed
48-camera fixture. Root's full suite ran 540 tests with eight skips before
the run. Output is on the external SSD at
`/Volumes/backups/code/crispmesh-data/mustard-sparse-masked-dense-003`;
[`failure evidence`](../tests/evidence/mustard-sparse-masked-dense-failures.json)
binds its result, verbose log, PLY and all 24 base DMAP SHA-256 values. No
held-out photos, reference mesh or GT determined the crop or reconstruction.

The prior uncropped 002 native run produced **one** base depth map with 20,460
positive masked pixels, but **zero** fused points. The cropped 003 native run
selected usable neighbor views for half the images and produced **24**
222×302 base depth maps. OpenMVS reported 304,933 fused depths and 24,956
points before its final 24,657-vertex `dense.ply` export. Densification
returned code 0 in 9.225 seconds, under the frozen 600-second, two-thread,
4-GiB child-RSS, 3-GiB output and dual 10-GiB free-space limits. Root's native
CTest activity overlapped the beginning of 003, so this timing is **not** a
controlled speed benchmark against 002.

The predeclared strict camera audit required a base DMAP for **all 48**
registered views. It correctly failed at **24/48**, before meshing. A separate
read-only diagnostic checked the 24 available maps: 460,545 positive depth
pixels, zero outside their native cropped masks, and maximum absolute K/R/C
differences of 0 / 2.22e-16 / 1.33e-15 against the cropped COLMAP cameras.
Those checks validate only the available depth-map coordinates and mask
application; they cannot certify missing views, whole-object coverage, camera
pose correctness or scanner geometry. The 003 result remains `failed`; its
partial dense PLY is research evidence, **not** a completed rough mesh or a
quality/SOTA result. There was no retry or threshold adjustment.

The next bottleneck is camera/neighbor coverage. The supplied image-estimated
poses are not independently validated as a physically correct orbit, and 003's
missing maps form broad view spans. In pinned OpenMVS v2.4.0, neighbor scores
are weighted by projected shared-point area; depth-map initialization then
uses an absolute and relative neighbor-score floor. This makes tiny image
support and incorrect cameras plausible explanations for the sparse neighbor
graph, but the current experiment does **not** isolate either cause. Any
future crop/pose intervention needs a separately frozen, bounded comparison;
these failed artifacts must remain intact. See the primary
[neighbor scoring source](https://github.com/cdcseacave/openMVS/blob/v2.4.0/libs/MVS/Scene.cpp#L1189-L1322),
[initialization source](https://github.com/cdcseacave/openMVS/blob/v2.4.0/libs/MVS/SceneDensify.cpp#L172-L230),
and [default thresholds](https://github.com/cdcseacave/openMVS/blob/v2.4.0/libs/MVS/DepthMap.cpp#L65-L91).
