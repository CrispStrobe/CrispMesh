# R02: masked sparse reconstruction

Implemented a sparse point/track stage from calibrated image poses. This is a CPU baseline and a geometry diagnostic, not a dense surface or a completed scan. Rendered-image acceptance passes; measured real-photo acceptance remains open.

## Execution contract

`crispmesh reconstruct-sparse <project.json>` re-estimates poses from the current project images in the same invocation. It does not trust an imported pose report with potentially stale calibration. Every image requires a maskPath: PNG with matching dimensions, 8-bit binary grayscale (0 excludes, 255 includes). Mask paths receive the same containment and size checks as source images. The board and stationary background must be excluded; automatically project and exclude known marker polygons as a second guard. A mask is not inferred from image alpha.

The implemented baseline uses single-scale ORB, at most 1,500 features per view, with a 32-pixel elliptical mask erosion (zero outside the image) and a 32-pixel image-border exclusion. This keeps descriptor/orientation/filter support inside the object; it does not provide multiscale feature robustness. Match only nearby entries in the manifest's acquisition order (pair window 2), using mutual nearest/ratio-filtered Hamming matches. Limits are 64 views, 256 million aggregate pixels, at most 125 candidate pairs and a 16 MiB output report. Image IDs are limited to 256 bytes and paths to 4,096 bytes. Reports record feature, pose, matching, geometry and resource settings.

Undistort matched image coordinates using the declared calibration. Triangulate in the board/object frame using actual per-image poses. Reject insufficient baseline/parallax, nonpositive depth, excessive pixel reprojection error and inconsistent tracks. A track contains at most one observation per image. Initial thresholds: minimum parallax 1 degree, maximum per-observation reprojection error 2 pixels, ratio 0.75. These are baseline settings, not measured scanner accuracy guarantees. Bundle adjustment is out of scope.

Report JSON fields:

- `ok:true`, `sparseSchemaVersion:1`, `projectId`, `units:"mm"`, `coordinateFrame:"board"`.
- `backend:{name:"OpenCV",version,feature:"ORB",settings:{...}}` with all numeric limits and algorithm settings.
- `views:[{imageId,path,maskPath,featureCount,rotation:[9],translationMm:[3]}]` in manifest order; poses remain object-to-camera and row-major.
- `points:[{id,positionMm:[3],rmsReprojectionErrorPx,maxReprojectionErrorPx,minParallaxDeg,observations:[{imageId,pixel:[x,y]}]}]` with a numeric ID and at least two distinct views per track; pixels refer to original distorted image coordinates.
- `statistics:{candidateMatches,acceptedTracks,rejectedMatches,...}`; additional counters may explain filtering. Accepted tracks equals points.length.

Failure emits one structured `ok:false,error:{code,message,imageId?}` object, nonzero CLI status and no partial success artifact. Missing masks, no usable features/matches, degenerate camera pairs and a zero-point output must not report successful reconstruction. The command and owned C ABI produce JSON without modifying source files or project stage state. Ordinary `reconstruct` stays unavailable until the full pipeline exists.

## Ownership and acceptance

Sol sparse-core agent owns core implementation/header/CLI and root CMake. Sol fixture agent owns `core/tests/sparse_integration.cpp` and any supporting `tests/sparse/` files, coordinates the public interface, and independently renders textured geometry, board markers and masks into multiple known views. Sol UI agent owns the sparse-report inspector and its validation/tests in `apps/desktop/`. Main agent owns milestone documentation, review and independent end-to-end verification.

Acceptance must exercise the production path from rendered images through marker detection, masked features, matches and triangulation. Check nonzero tracks, metric geometry/reprojection errors, exclusion of intentionally textured background/board, mask failures and negligible-baseline rejection. Keep clearly labeled generated artifacts under the build tree so results can be independently inspected. UI imports results for inspection only, validates project/image identity and finite geometry, and clears imported results on edits. Real-photo accuracy remains a separate gate requiring a measured dataset.

The current three-view fixture yields 1,189 points with a 2.05 mm median and 5.70 mm 95th-percentile depth error against two known textured planes. Acceptance requires at least 25 points, 90% near the known panel bounds, coverage of both planes, at least 10 three-view tracks, median depth error below 3 mm and 95th percentile below 8 mm. These fixture-specific thresholds are not product accuracy claims. Tests also cover missing, nonbinary, dimension-mismatched and escaping masks, empty feature support and degenerate camera baseline. Artifacts and `quality-report.json` are retained under `build-opencv/synthetic-sparse-fixture/`.

The desktop inspector renders up to 20,000 sampled points while calculating statistics over the complete imported report. It validates report structure and project identity, not provenance or real-world accuracy. No source images, masks or stage metadata are changed by inspection.
