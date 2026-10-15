# Sealed TRAIN verified-track receipts

The first two attempts produced no receipt. Their destination
`/Volumes/backups/code/crispmesh-data/mustard-verified-tracks-001.json`
remains absent. After the separately approved scalar-only audit identified the
exact empty-row encoding, one narrowly corrected, fresh counts-only run wrote
`mustard-verified-tracks-002.json`. No feature coordinates or tracks were
exported, and no fixed-board-pose sparse or dense integration ran.

The first root-approved attempt used receipt runner SHA-256
`6cd6442e25dc96e8d84af5dcc7e138a0468e84b04bb9d5f2c16c64f0537c46cc`
and exited in 0.77 seconds before writing output. Its supervisor suppressed
the child error, so no exact cause was recorded. After a synthetic-tested
bounded error-reporting patch, the one approved retry used runner SHA-256
`db53413fbd46dc419059df1025f4ed602cc106fd706feedf3f573fef8c2c938a`
and exited in 1.83 seconds with the sanitized diagnostic
`TrackAdapterError: invalid verified geometry row`. No third attempt was run.

The subsequently authorized scalar-only audit identified the exact mismatch:
728 of 1,128 geometry rows have `rows=0, cols=2, data=NULL, config=0`; the
other 400 positive rows have exact blob lengths. Every row has `cols=2`.
Configuration counts were 728 at 0, 202 at 3, and 198 at 6. The audit's
first five offending pair IDs were 2147483657 through 2147483661, all with
the same zero/2/NULL/0 shape. Audit runner SHA-256 was
`3348e256f0605371ca14bef6d02f1dbae62fe26c7c12ce10545dba5a48820307`;
the DB hash remained unchanged. No feature blobs or poses were selected.
This does **not** establish whether the positive-row correspondences are
usable or whether any object-only track exists. A narrow adapter correction
for this exact empty-row encoding was synthetic-tested and approved before
the fresh `-002` receipt run.
The accepted coarse masks may contain rotating board or
support; mask-supported candidates would not, by themselves, be object-only
geometry.

Preflight and postflight source SHA-256 remained:

- Stage `stage-report.json`: `bbcba624f6a51f82e0ebdebf6e7e4cda10804bc8df34cfd0d5b3242983a225d0`
- Producer `result.json`: `5a83c3e05e2b66887411799ecc8c5f3f41e3b8af3bbd775d308055ef7ee0041e`
- Sealed database: `5858be16dfa3f0fc50303d0eb4a001e21ee14036f67755a9997d5dac374b3075`

Both internal and external volumes remained above the 10 GiB reserve after
the second attempt (approximately 22.2 GiB and 14.2 GiB free, respectively).

The approved `-002` diagnostic completed in 8.69 seconds. Its 1,102-byte
[external receipt](</Volumes/backups/code/crispmesh-data/mustard-verified-tracks-002.json>)
has SHA-256 `a374019ff3cd6c211ddacaf270eccbbd690cbe9530d194d8cf4ee65c1b8fc898`.
It binds adapter SHA-256
`0ff9d78b055e8c5b5f806dbe8fb86a9501845154b4f42f0ef4e6f4a9017bfc7a`,
runner SHA-256
`01f136b9b1f61d9ec0d14ac901aaebb773727a769adb5c77e329171cba61f18d`,
and exact input-manifest SHA-256
`266190342e7d807be19f3b791409c88740702ca1cc773cefde98270c0aaa2ca5`.
The stage, producer, and database hashes above remained unchanged post-run;
internal/external free space remained approximately 22.2/13.6 GiB.

| Diagnostic count | Value |
| --- | ---: |
| TRAIN images | 48 |
| Geometry rows / skipped empty rows | 1,128 / 728 |
| Verified edges / mask-supported edges | 12,152 / 11,863 |
| Connected components / candidate ≥3-view tracks | 1,013 / 518 |
| Rejected duplicate-image / oversized / short components | 81 / 3 / 411 |

The 518 tracks are **candidates under coarse masks only**. The checkerboard
and rotating support may be included in those masks; this count is not an
object-only quality result and is not approval to export tracks or run sparse
or dense reconstruction.
