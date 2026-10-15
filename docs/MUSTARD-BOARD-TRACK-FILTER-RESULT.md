# Mustard board-frame candidate-track filter: counts-only result

Status, 2026-09-27: **249/518 candidate tracks retained by frozen geometric gates; not object-only proof.** One root-approved, supervised read-only extraction/filter pass produced only the 3,216-byte [counts receipt](</Volumes/backups/code/crispmesh-data/mustard-board-track-filter-001.json>), SHA-256 `eaf968faf4d7d0e95e00ef091f7727f34c5c4026f2a1995ac5a147f4d5fc5be2`. Runner SHA-256 was `e7ec7e9543cf0316e050691d5bc65a80b3b9dd3baeae3c8420411757ad12c42a`; it used the frozen 39 capture-calibration-assisted board poses, the sealed verified-index TRAIN tracks, and coarse support masks. It wrote **no feature coordinates, 3D points, virtual RGB/masks, sparse model, or dense output**. No Berkeley per-angle pose, scanner mesh, sensor depth, held-out image, or reference score entered the filter.

| Whole-track result | Count |
| --- | ---: |
| Input mask-supported candidates | 518 |
| Fewer than three posed views after removing unposed observations | 115 |
| Failed frozen raw/virtual reprojection ≤2 px | 153 |
| Near checkerboard plane, outside printed footprint | 1 |
| Accepted under frozen gates | **249** |

The reason counts conserve all 518 tracks. The filter dropped 842 observations from the nine unposed frames; the 249 retained tracks carry 1,089 observations across 38 of 39 posed frames (`NP3_090` contributes none). The frozen ≥8-track minimum is met. Among retained tracks, signed camera-facing height has median 4.96 square units (p95 6.73), maximum pairwise viewing-ray parallax median 18.84° (at least one baseline, not all pairs), and per-track maximum distorted reprojection median 0.65 px (p95 1.61). None were recorded as `board_plane_footprint` rejections; this does **not** establish absence of board/support leakage, because elevated support or mis-triangulated board features can survive the height rule. The coarse masks were not object silhouettes.

Post-run independent SHA-256 checks matched the calibrated pose report `25f37be625cf4edd00bdf29c479529d341cd76d3dc748d8b966e482c4668619d`, earlier 518-track count receipt `a374019ff3cd6c211ddacaf270eccbbd690cbe9530d194d8cf4ee65c1b8fc898`, staged TRAIN/mask report `bbcba624f6a51f82e0ebdebf6e7e4cda10804bc8df34cfd0d5b3242983a225d0`, producer report `5a83c3e05e2b66887411799ecc8c5f3f41e3b8af3bbd775d308055ef7ee0041e`, and database `5858be16dfa3f0fc50303d0eb4a001e21ee14036f67755a9997d5dac374b3075`. The worker's sampled RSS peaked at 164,560 KiB, below the 2 GiB cap; output was below 2 MiB and the run completed within 120 seconds. Both disks remained above 10 GiB free (post-run recheck: internal 23,200,040 KiB; external 14,179,644 KiB). The original source artifacts were not mutated.

This is a conservative **candidate** pool in arbitrary checker-square coordinates. Before an object-only fixed-pose sparse handoff, a separate image-derived 3D-distribution and bottle-versus-board/support QA gate is required; the counts receipt alone is not authorization for virtual-image staging, sparse export, or dense reconstruction.
