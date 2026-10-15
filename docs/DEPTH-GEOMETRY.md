# R03b geometry and fusion experiment

`core/depth` is a separate OpenCV C++20 test library. It consumes already known, calibrated object-to-camera poses (`Xcamera = R Xobject + t`, row-major R, millimetres). It does not estimate poses, compute image disparities or enable full reconstruction in the product.

`rectify` composes a relative camera transform, calls calibrated OpenCV stereo rectification with the declared radial/tangential distortion, and returns floating-point remap grids, both rectified projection matrices, Q, the matching axis, and signed disparity bounds from a caller-provided conservative near/far interval. Both horizontal and vertical baselines are supported. Invalid calibration, unequal image dimensions, baseline below 0.001 mm and non-axis-aligned rectified translation return structured status/reason. Pixel disparity uses `coordinate_in_first - coordinate_in_second` on the reported axis. Bounds include a one-pixel quantization margin. This is only a geometric search interval; callers must still limit image support and matcher memory.

`triangulate_object` undistorts original image pixels, triangulates with the full relative pose, checks parallax at least 0.1 degree, positive depth in both cameras, and reprojection within 2 pixels, then maps the result into the moving board/object frame. `fuse_tracks` checks finite positive axial depth and an object mask at the projected nearest pixel in at least two distinct camera centers. A nearer observed surface marks a view as occluded; an observed farther surface indicates free-space conflict and vetoes the candidate. Accepted candidates sharing a supplied track ID are confidence-weighted within a distance gate; adjacent tracks remain separate. This intentionally does no spatial smoothing, hole filling, normal estimation, dense occlusion reasoning, or mesh generation. Depth samples must already be produced by a real masked matcher; the function does not validate that the matcher excluded background within its support window.

The root CMake now adds `core/depth` inside its OpenCV test conditional. Configure/build/test:

```sh
cmake -S . -B build-opencv -DCRISPMESH_WITH_OPENCV=ON -DCRISPMESH_BUILD_TESTS=ON
cmake --build build-opencv --target crispmesh_depth_geometry_tests crispmesh_depth_image_tests crispmesh_depth_image_fusion_tests -j 4
ctest --test-dir build-opencv -R 'depth_(geometry|image|image_fusion)_synthetic' --output-on-failure
```

The geometry executable emits one JSON line with candidate counts and local elapsed milliseconds. Its fixture uses three analytic camera views, known 500 mm points, independent pixel-parallax calculations, distortion, rotated and translated cameras, horizontal and vertical baselines, masks, occlusion/free-space conflict, a behind-camera sample, a disconnected mask centroid, and two adjacent distinct tracks.

The image executable renders two textured analytic surfaces into three calibrated views. It runs StereoSGBM on two rectified pairs, derives points from both disparity maps using calibrated projection, and accepts only points agreeing within 6 mm in the shared object frame. Ground truth is used only after matching for scoring. Its report separates requested, foreground-eligible, valid per pair, common-valid and cross-pair consistent counts, and per-pair depth errors before/after consistency filtering. Each CTest run writes source/rectified PNGs, masks, signed-16 disparity PNGs and `report.json` in a new directory below `build-opencv/depth-image-fixtures`; disparity PNG integer values encode `32768 + disparity_pixels * 16`. The test checks at least 10 GiB plus estimated output allowance remains free before saving.

The original `depth_image_synthetic` fixture applies its object mask after SGBM and uses the two pair estimates directly. The separate [image-to-fusion experiment](DEPTH-IMAGE-FUSION.md) masks matcher inputs, checks left/right disparity, and sends image-derived candidates and two independently matched depth views into `fuse_tracks`.

These are synthetic geometry and rendered-image regressions, not evidence of photographed scan accuracy or production performance. No measured rig calibration or photographs are assumed.
