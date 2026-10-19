# Silhouette-guided second camera pass (experimental)

This optional path starts from an **own photo-recovered** scene and its basic
silhouette mesh. It adjusts camera extrinsics against visible mesh contours
and the actual measured SfM feature observations. It retains the original
camera path and does not change app defaults.

```sh
python -m scripts.camera_refinement.silhouette_refine \
  --inputs /data/own-inputs --mesh /data/own-silhouette.stl \
  --model /data/own-colmap-model --photo-map /data/frontend/photo-map.json \
  --output /data/second-pass-inputs
crispmesh-dense run --inputs /data/second-pass-inputs --output /data/second-pass \
  --base-surface silhouette --splats capture
```

Use the same PyCOLMAP 3.11 scientific environment as the camera-recovery tools.
Only the caller's own recovered poses, sparse points, feature observations and
masks are permitted. The supplied model's world frame and source-photo mapping
must match the prepared scene; the tool verifies pose agreement before fitting.

The CPU renderer identifies visible silhouette samples on the actual mesh.
Bounded least squares combines contour distance, measured sparse reprojection
and a pose prior. Focal lengths, distortion and sparse 3D structure stay fixed:
this is **camera-only refinement, not full joint bundle adjustment**. Rotation
components are bounded to five degrees and translation components to 5% of the
own sparse radius. Camera pivots are near the object to avoid unintended large
translation when rotating.

Every fifth camera stays unchanged and its silhouette does not enter pose
fitting. Every fifth feature observation of fitted cameras is also excluded
from the fitting residual. An update needs improved contour distance, no loss
of the rasterized silhouette IoU, and limited held-out feature error increases
(median <= 0.25 px; p95 <= 0.5 px, both at the normalized measurement resolution).
`--feature-weight` changes the feature constraint (default 10); `--views` permits
a small named-view diagnostic, and `--hold-every` changes the camera holdout.
The report includes the measurement image size and every accepted/rejected step.
Photos and masks are linked into a separate scene; originals remain intact.

For a controlled comparison, build the initial hull from fitting views only,
then rebuild both final hulls and Gaussian models using those same fitting views.
Evaluate through original frozen reserved cameras. This tool cannot verify which
photos built its supplied hull. Initial SfM/preprocessing still used the full
batch in the control below: it is stage-level validation, not a wholly unseen
reconstruction. Lower in-fit silhouette error can reinforce a wrong hull.

Initial iPad diagnostics show the tradeoff: fitting a silhouette more strongly
reduced a selected view's contour error from 6.22 to 5.14 pixels but increased
held-out sparse error from 0.63 to 1.82 pixels; that step was rejected. With the
stronger feature constraint, the full pass accepted 84 small camera updates,
held 22 cameras fixed and took 65.2 seconds. Median rotation was 0.159 degrees,
maximum 1.906 degrees. That diagnostic used the all-view hull.

A stricter control reserved 22 of 106 own recovered views. Both hulls and both
Gaussian runs used only the other 84 views. The pass accepted 83 camera updates
in 55.3 seconds; median/max rotation was 0.140/2.078 degrees. Gaussian took
227.2 seconds before and 270.6 after on a shared M1. One baseline attempt was
restarted after a disk stop; these are successful-stage times.

Reserved-view median mask IoU changed 0.87519 → 0.87609. The meshes remain very
similar, with the middle-view mismatch still visible. On 11 matching reserved
RGB photos, two paired PSNR values improved and nine worsened (mean −0.091 dB,
median −0.135 dB). Aggregate medians increase 22.968 → 23.076 dB because they
select different views; that does not describe the paired trend. One training
per path cannot quantify GPU numeric variation. No clear improvement was found.

Jointly refining cameras, lens parameters and 3D structure with image evidence
is the next hypothesis. This version fixes the latter two and stays optional.

This is original project code using the existing project renderer and permissive
NumPy/SciPy/OpenCV dependencies. No upstream algorithm implementation was copied.
For a broader alternative, [Photometric Bundle Adjustment for Vision-Based
SLAM](https://arxiv.org/abs/1608.02026) describes jointly optimizing cameras and
structure from image intensities; that is not the algorithm implemented here.
