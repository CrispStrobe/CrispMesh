# Sealed verified-index database to candidate object tracks

Status: synthetic-only adapter and tests. No sealed TRAIN database, real mask,
or checkerboard pose was opened by this adapter; no live extraction is approved.

## Frozen source and provenance boundary

Proposed first source for a later approved run is the sealed
`/Volumes/backups/code/crispmesh-data/mustard-sfm-masked-fixed-exhaustive-001/database.db`
(SHA-256 `5858be16dfa3f0fc50303d0eb4a001e21ee14036f67755a9997d5dac374b3075`,
1,851,392 bytes), whose producer `result.json` is SHA-256
`5a83c3e05e2b66887411799ecc8c5f3f41e3b8af3bbd775d308055ef7ee0041e`.
The 48 TRAIN RGB and **coarse pose-support** masks are staged in
`/Volumes/backups/code/crispmesh-data/mustard-sfm-train-001` under
`stage-report.json` SHA-256
`bbcba624f6a51f82e0ebdebf6e7e4cda10804bc8df34cfd0d5b3242983a225d0`.
These are prior sealed-provenance facts, not new checks made by a live run.
Before use, a separate caller must bind the exact 48 image names, JPEG hashes,
PNG hashes, widths and heights from the sealed stage report into `ImageSource`
entries and independently verify the report/producer hashes. The adapter
rechecks every listed source hash and exact directory inventory, and the
database hash before and after extraction. No SQLite sidecar is accepted.

The adapter uses only `images`, `keypoints`, and verified
`two_view_geometries` feature-index pairs, accessed with immutable read-only
SQLite. It ignores raw `matches`, saved sparse points, folded SfM cameras, and
all supplied object-dataset poses/depth/reference geometry. It never edits or
copies the source DB. Keypoint blobs are bounded float32 COLMAP coordinates;
the output subtracts 0.5 on each axis to match the fixed-board-pose ingester's
OpenCV integer-center convention. Only in-bounds keypoints on nonzero mask
pixels enter candidate edges. This is **mask-supported**, not proof of a
physical object-only track: coarse masks may include board/support, so a
visual/semantic mask review remains necessary before dense use.

The separate scalar-only audit found 728 COLMAP zero-match rows encoded as
`rows=0, cols=2, data=NULL, config=0`. These exact rows are now counted as
`empty_geometry_rows` and skipped. A positive row still requires a byte blob
of exactly `rows*8`, `cols=2`, and positive configuration; malformed zero
rows, including `data=b''` with positive configuration, fail closed.

## Deterministic graph rule and caps

Decode pair ID to ascending image IDs, sort pair rows, and use the stored blob
order of unique uint32 verified index pairs. Deterministic union-find joins
feature nodes `(image_id, keypoint_index)`. Reject an entire connected
component if it contains two distinct features from one image or grows beyond
24 nodes; never split an ambiguous component by a preferred edge. Accept only
clean components with at least three distinct views. Track IDs and observation
order are deterministic. The 24-node cap is a conservative alias guard; it
will discard legitimate tracks observed in more views, rather than silently
making them safe.

Fail-closed resource limits are: exactly 48 frames in production; database
at most 20 MiB; at most 10,000 keypoints per image, 1,128 geometry rows,
1,000,000 verified edges, 200,000 graph nodes, 24 nodes per accepted
component, and 60 seconds wall time. No output artifact is written. A later
approved live diagnostic should use a supervisor timeout as well as the
in-process clock, both disks above 10 GiB, and a fresh bounded receipt path.
It must report source/runner hashes and counts of verified edges, mask-supported
edges, clean tracks, short components, duplicate-image conflicts, and oversized
components; it must not tune caps from those counts.

The resulting candidate `ObjectTrack`s can be passed to the separate
`board_pose_sparse_contract` only after independently approved, sealed
checkerboard poses and intrinsic assumptions are reviewed. That contract then
triangulates under fixed board-frame poses and applies its own cheirality,
parallax, and reprojection gates. No full-DB extraction, model export, or dense
reconstruction is authorized by this plan.
