# Native foundation

`crispmesh_core` is a C++20 library with a versioned C ABI in
[`include/crispmesh/core.h`](include/crispmesh/core.h). Its ABI version and project
schema version are both 1. Callers provide output buffers; pass `NULL, 0` to
query the required byte count, which includes the NUL terminator. A successful
size query returns `CRISPMESH_BUFFER_TOO_SMALL`. The second call returns the
operation status and a single UTF-8 JSON object. The library retains no input
pointers. The pose operation also offers `crispmesh_estimate_poses_alloc`, which
computes once and returns a caller-owned string released with `crispmesh_free`.

The CLI commands are:

```text
crispmesh capabilities
crispmesh validate <project.json>
crispmesh diagnose-geometry
crispmesh estimate-poses <project.json>
crispmesh reconstruct-sparse <project.json>
crispmesh reconstruct <project.json>
```

Results are one JSON object on stdout. Exit code 0 means success, 2 means an
invalid project or file error, 3 means reconstruction is unavailable, and 64
means incorrect CLI usage. `reconstruct` validates its input, then reports
`unavailable`; it never writes a mesh. `capabilities` is the source of truth
for currently integrated stages. `diagnose-geometry` projects a synthetic
point into two cameras, triangulates it, and reports position and reprojection
errors. It does not claim to reconstruct photographs.

`estimate-poses` is available in an opt-in OpenCV build. It reads actual PNG or
JPEG pixels and returns one JSON report with `poseSchemaVersion:1`, `projectId`,
backend/version/configuration, and one entry per image. Each entry includes the
image ID and relative path, detected and ignored marker IDs, row-major 3×3
rotation, translation in millimetres, and RMS/maximum reprojection errors in
pixels. It exits with a structured error if any image fails; it does not write
to the project or claim a completed stage from partial views. Only
`opencv-radtan` distortion with 4, 5 or 8 coefficients and `DICT_4X4_50`
ArUco layouts are supported. The fixed residual limits are 3 px RMS and 8 px
maximum. The report records the OpenCV detector defaults and the 0.25 px
minimum separation required between planar pose candidates; near-equal
solutions are reported as ambiguous.

The v1 project contract is in [`../docs/PLAN.md`](../docs/PLAN.md) and
[`../docs/project.schema.json`](../docs/project.schema.json). Coordinates are
right handed and measured in millimetres. Camera coordinates have x right,
y down, z forward. Poses are object-to-camera transforms `Xc = R Xo + t`,
with row-major matrix storage. Board corners are decoded marker corners in
top-left, top-right, bottom-right, bottom-left order; board coordinates can be
rotated in plane but must have positive winding and z=0. Calibration intrinsics
belong to the original image dimensions. Pose estimation rejects mismatched
dimensions, and JPEG EXIF orientation is ignored so the pixels must already be
in the calibrated orientation. Input photographs remain immutable.

Validation checks UTF-8 JSON syntax, duplicate keys, required fields, allowed
stage names and values, unique image IDs, finite calibration values, rotating
board geometry, and lexical relative paths. Extra project metadata fields are
permitted. Path validation rejects traversal and Windows/Unix absolute forms;
the pose operation additionally checks canonical symlink containment, image
existence, size, encoded dimensions before decoding, and residual quality. It
limits input files to 64 MiB, images to 40 megapixels and 10000 pixels per
axis, and board coordinates to one million millimetres. A valid manifest is
not necessarily ready for pose estimation or reconstruction.

Build and test:

```sh
cmake -S . -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build
ctest --test-dir build --output-on-failure

cmake -S . -B build-opencv -DCMAKE_BUILD_TYPE=Release -DCRISPMESH_WITH_OPENCV=ON
cmake --build build-opencv
ctest --test-dir build-opencv --output-on-failure
```

The opt-in integration test preserves a labeled synthetic project, rendered
ArUco PNG and CLI report under `build-opencv/synthetic-pose-fixture/`. It checks
the detector-to-PnP path and several failure cases. It does not establish
accuracy on measured photographs.

`reconstruct-sparse` is available in the same opt-in OpenCV build. It
re-estimates all poses from the source images on each run, requires an 8-bit
binary grayscale PNG `maskPath` for every image, and emits a board-frame sparse
point/track report with original distorted observation pixels. The C ABI entry
point is `crispmesh_reconstruct_sparse_alloc`; release its result with
`crispmesh_free`. No project file or image is modified. A failed view or zero
valid points produces one structured error and a nonzero CLI exit status.

The baseline uses deterministic single-scale ORB (up to 1,500 features per
view), a 32 px elliptical mask erosion, and projection-based exclusion of all
configured board markers. It matches acquisition neighbors within a two-entry
window using mutual 0.75-ratio Hamming matches. Tracks hold at most one
observation per view. Ray triangulation requires at least 1 degree of parallax,
positive camera depth, and at most 2 px distorted reprojection error for every
observation. It does not include bundle adjustment or scale robustness.
Execution is capped at 64 views, 256 million aggregate source pixels, 125
candidate pairs, 64 MiB per source/mask file, and a 16 MiB JSON report. The
report's `backend.settings` and `statistics` fields expose these thresholds and
filter counts. `reconstruct` remains unavailable because this stage does not
produce a dense surface or mesh.
