# Bounded sealed TRAIN verified-track receipt

Status: two bounded attempts at `-001` failed without a receipt; the narrow
zero-row correction was reviewed and one fresh `-002` counts-only diagnostic
completed. Its receipt path was:
`/Volumes/backups/code/crispmesh-data/mustard-verified-tracks-002.json`.

The runner pins the prior sealed exhaustive fixed-initial masked PyCOLMAP
database SHA-256
`5858be16dfa3f0fc50303d0eb4a001e21ee14036f67755a9997d5dac374b3075`,
producer `result.json` SHA-256
`5a83c3e05e2b66887411799ecc8c5f3f41e3b8af3bbd775d308055ef7ee0041e`,
and TRAIN stage `stage-report.json` SHA-256
`bbcba624f6a51f82e0ebdebf6e7e4cda10804bc8df34cfd0d5b3242983a225d0`.
It validates the actual `mustard_train_only_stage_v1` fields (`train_names`,
`train_photo_sha256`, `cleaned_masks[*].sha256/kept_pixels/ignored_pixels`,
name-list hash, completed copy status, and explicit coarse-mask role) against
the `classical_backend_v1` producer's ordered `inputs` and `pose_masks`,
image-only SfM source, and fixed-initial status. The stage report does not
carry dimensions: the runner derives these from each hashed staged JPEG and
checks the mask-pixel counts against its area. The adapter then independently
checks exact source inventories, hashes, dimensions and binary masks.

The public command accepts only a fresh JSON path directly on the external
data volume. It first checks both the repository and external volumes retain
at least 10 GiB free (including a 2 MiB output allowance), then runs the
read-only extraction in a child with a 75-second supervisor timeout; the
adapter itself has a 60-second wall and bounded graph/DB caps. No receipt is
written if the child fails, times out, or exceeds output/log limits. The
receipt is under 2 MiB and contains counts (including skipped empty geometry
rows), source/runner/manifest hashes,
and the board/support leakage caution only—no feature coordinates, tracks,
camera poses, scanner/depth/reference data, or reconstructed geometry.

The approved run used the pinned
`.local-tools/colmap-sparse/venv/bin/python -m scripts.object_motion.mustard_verified_track_receipt`
with `--output` set to the fresh path above, `PYTHONDONTWRITEBYTECODE=1`,
and verified the resulting file SHA-256, status, source/runner hashes, and both
disk floors afterward. Do not tune union thresholds from the result. Any
candidate track count is **not** an object-only quality metric: accepted
coarse pose masks may include the checkerboard or rotating support. No
checkerboard pose or sparse/dense model enters this receipt diagnostic.
