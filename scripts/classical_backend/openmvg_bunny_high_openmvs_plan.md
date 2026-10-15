# OpenMVG HIGH bunny → OpenMVS continuation

This is a **composed** pipeline: the image-only OpenMVG HIGH sparse result is
one run, and `openmvg_bunny_high_openmvs.py` is a separate OpenMVS continuation.
Do not describe its dense or mesh runtime as an independent end-to-end result.

The continuation currently abstains. The sparse producer is still pending an
independent camera geometry review, and the official
`openMVG_main_openMVG2openMVS` converter is not built. The existing frozen
OpenMVG v2.1 Ninja graph contains that target, but no binary exists yet. No
dense or mesh work was launched while preparing this adapter.

## Gate

The reviewer must first inspect the new HIGH model in original numeric
`bunny_N_rgb.png` order, using the sealed `prepare-manifest.json` mapping.
`frame_####.png` order is lexicographic source-file order and is not camera
sequence. Review trajectory, intrinsics, sparse points on the foreground,
reprojection quality, and whether the reconstructed object is the bunny.
73 registered poses alone do not pass this gate. Scanner geometry and supplied
poses may be used only in explicitly post hoc diagnostics, not as pipeline
inputs or tuning targets.

Only after that review may a separate JSON receipt be issued with exactly:

```json
{
  "schema": "bunny_independent_camera_review_v1",
  "decision": "approved_for_openmvs",
  "camera_geometry_reviewed": true,
  "object_region_consistent": true,
  "reviewer": "name or review identifier",
  "source_receipt_sha256": "SHA-256 of sparse receipt.json",
  "sparse_model_sha256": "SHA-256 of sparse/sfm_data.bin",
  "prepare_manifest_sha256": "SHA-256 of prepared photo mapping"
}
```

The adapter separately requires the receipt file's exact SHA-256 on the CLI.
It checks the sparse producer's status, all 73 original and staged processed
photo hashes, the model hash, and the numeric source-frame mapping before
creating an output directory. The gate is an explicit review decision, not an
automatically inferred property of pose count or reprojection residual.

## Tool and resource plan

After gate approval, build only `openMVG_main_openMVG2openMVS` from the
existing frozen OpenMVG v2.1 graph, after checking its dry-run closure and
available SSD headroom. The converter writes `scene.mvs` and 73 undistorted
images. OpenMVS then runs densify at resolution level 3 (max 1280 px), mesh,
refine at level 2, and texture. There is no scanner or ground-truth input.
No native masks are supplied by this direct OpenMVG export; foreground quality
must be checked in the resulting point cloud and mesh.

The adapter enforces two CPU threads, a 20 minute total deadline, a 4 GiB
sampled child RSS cap, a 16 MiB log cap per stage, a 1 GiB output cap, and an
11 GiB free-space floor on the output and workspace volumes. It checks the
source and tool hashes again after the run. If the disk headroom, converter,
or camera gate is absent, it exits before creating output.

Example, only after a positive camera receipt exists and its SHA is known:

```sh
python3 -m scripts.classical_backend.openmvg_bunny_high_openmvs \
  --source-receipt /Volumes/backups/code/crispmesh-data/openmvg-bunny-high-photo-sfm-001/receipt.json \
  --camera-gate /absolute/path/to/independent-camera-review.json \
  --camera-gate-sha256 REVIEW_RECEIPT_SHA256 \
  --expected-images build-opencv/bunny-gamma05-clahe2 \
  --openmvg-binary /Volumes/backups/code/crispmesh-data/openmvg-v21-ligt-off-oracle-001/build/Darwin-arm64-Release/openMVG_main_openMVG2openMVS \
  --openmvs-binary-dir .local-tools/classical-backend/bin \
  --output /Volumes/backups/code/crispmesh-data/openmvg-bunny-high-openmvs-001
```

The selected output must be a fresh directory name. The final `result.json`
records stage commands, counts, hashes, limits, and a false product-quality
flag; visual shape and camera checks remain separate evaluation tasks.
