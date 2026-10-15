# Scalar-only sealed two-view geometry row audit

Status: the one approved stdout-only audit completed without an output file.
This was a separate diagnosis of the failed verified-track receipt, not a
retry, parser relaxation, or track extraction.

Pinned input is
`/Volumes/backups/code/crispmesh-data/mustard-sfm-masked-fixed-exhaustive-001/database.db`,
SHA-256 `5858be16dfa3f0fc50303d0eb4a001e21ee14036f67755a9997d5dac374b3075`.
The audit checks the hash before and after, rejects SQLite sidecars and linked
or oversized files, opens `mode=ro&immutable=1`, and sets `query_only=ON`.
Its **only** SELECT is `pair_id, rows, cols, length(data), config` from
`two_view_geometries`, ordered by pair ID and limited to 1,129 scalar rows so
the 1,128-pair cap fails closed. It never selects blob bytes, feature indices,
keypoints, images, cameras, points, poses, masks, or depth.

Output is one JSON object on stdout, capped at 2 MiB. It contains aggregate
configuration counts, zero/positive/invalid row counts, exact row-count and
column-count histograms, blob-length exact/null/mismatch counts, total rows
violating the current adapter's scalar expectations, and at most the first
five offending pair IDs with `rows`, `cols`, `blob_length`, `config`, and
violation labels. No per-feature values are emitted. Both internal and
external volumes must have at least 10 GiB free before and after; the command
has a 30-second in-process alarm. No output file or other external artifact
is written by the audit.

Proposed exact command from the repository root, after independent approval:

```sh
PYTHONDONTWRITEBYTECODE=1 .local-tools/colmap-sparse/venv/bin/python -m scripts.object_motion.mustard_geometry_schema_audit
```

The scalar evidence was reviewed before the separate, narrow empty-row
adapter correction and fresh `-002` receipt authorization. The audit itself
did not authorize a track extraction. The prior coarse masks may include board/support, so any eventual
mask-supported tracks remain candidates rather than proven object geometry.

The audit's “offending” label reflects the **pre-correction** adapter rule.
The approved audit found 1,128 rows: 728 zero-match `data=NULL, config=0`
rows and 400 positive rows with exact blob lengths; all had `cols=2`.
Configurations were 0:728, 3:202, 6:198. The first five offending pair IDs
were 2147483657–2147483661, each zero/2/NULL/0. Database SHA-256 stayed
`5858be16dfa3f0fc50303d0eb4a001e21ee14036f67755a9997d5dac374b3075`,
both disk floors held, and no feature indices or blobs were selected.
