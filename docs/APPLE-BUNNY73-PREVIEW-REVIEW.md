# Apple Object Capture preview on the 73 processed bunny photos

Apple's `.preview` Object Capture run completed in **70.041 seconds** using the
same 73 contrast-preprocessed `frame_####.png` image bytes as the bunny
MVE/COLMAP comparisons and OpenMVG HIGH run. The launcher verified all input
hashes unchanged afterward. Its [USDZ](/Volumes/backups/code/crispmesh-data/apple-bunny73-preview-001-output/model.usdz)
is 1,212,586 bytes, SHA-256
`2dbefc3216132134124068fdceb9cf4eb43e5d922c59b47db750086042915d60`.
The [launch receipt](/Volumes/backups/code/crispmesh-data/apple-bunny73-preview-001-output/result.json)
records the exact inputs, compiled probe hash, limits, and timing. This is an
evaluation-only Apple result.

The existing bounded USDZ mesh reviewer decoded one mesh and generated an
[untextured PLY](/Volumes/backups/code/crispmesh-data/apple-bunny73-review-001/mesh-untextured.ply)
and [three-view neutral preview](/Volumes/backups/code/crispmesh-data/apple-bunny73-review-001/preview-neutral.png).
It found **7,431 vertices and 14,837 triangles**, all finite with valid
indices, no repeated-index triangles, and no zero-area triangles. Its bounding
box extents are 0.3527 × 0.3538 × 0.2685 in authored USD units; the authored
`metersPerUnit = 1` is not independent evidence of metric scale. The XY
preview shows a recognizable rabbit body, feet, and ears. The XZ and YZ
silhouettes are less informative and irregular. This view is a surface sample,
not a shaded or textured render.

The [bounded texture and topology review](/Volumes/backups/code/crispmesh-data/apple-bunny73-texture-review-002/texture-review.json)
found one face-connected component containing all 14,837 triangles, **25
boundary edges**, and no edges with more than two incident faces. There is no
detached mesh component or obvious separate support disk in the neutral
preview. A connected mesh can still absorb background or support surfaces;
the 25 boundary edges also mean it is not closed. The USDZ has one 1024 × 1024
RGB PNG atlas and a material binding. It has 8,464 UV points and **44,511
valid UV indices**, one for every triangle corner. The [atlas preview](/Volumes/backups/code/crispmesh-data/apple-bunny73-texture-review-002/texture-atlas-preview.png)
contains recognizable photographic texture, with blurred regions and irregular
seams. UV completeness establishes that the atlas is attached, not that the
texture looks good on the rendered model.

For context, the finalized MVE bunny mesh has 589,429 vertices and 1,193,446
faces; the prior masked COLMAP/OpenMVS textured bunny mesh has 20,690 vertices
and 41,283 faces. Apple's `.preview` output is much coarser by triangle count.
Those counts reflect different detail settings and cleanup, and do not rank
shape quality. The [MVE preview](.local-tools/mve-full-bunny-shape-preview.png)
shows substantial shape distortion, while the [masked OpenMVS preview](.local-tools/classical-bunny-masked-refined-primary-preview.png)
shows a bunny-like form with irregular regions. Those previews use different
view alignment and include a post hoc scanner reference; the Apple neutral
preview is in its native orientation. No common pose alignment, surface
distance, or blinded visual scoring was performed here, so no winner or
shipping-quality claim follows.

To reproduce the read-only reviews without copying the USDZ, use the project
Python environment with NumPy and Pillow:

```sh
.local-tools/colmap-sparse/venv/bin/python -m scripts.apple_object_capture.review_mesh \
  --source-usdz /Volumes/backups/code/crispmesh-data/apple-bunny73-preview-001-output/model.usdz \
  --run-dir FRESH_MESH_REVIEW_DIR

.local-tools/colmap-sparse/venv/bin/python -m scripts.apple_object_capture.review_bunny_texture \
  --source-usdz /Volumes/backups/code/crispmesh-data/apple-bunny73-preview-001-output/model.usdz \
  --launch-receipt /Volumes/backups/code/crispmesh-data/apple-bunny73-preview-001-output/result.json \
  --mesh-receipt FRESH_MESH_REVIEW_DIR/result.json \
  --review-dir FRESH_TEXTURE_REVIEW_DIR
```

The texture reviewer bounds USDZ, decoded USD, atlas, thumbnail, and time to
decode with `usdcat`; it verifies that the launch receipt, mesh receipt, and
USDZ hashes match. Both reviewers leave the reconstruction artifact untouched.
