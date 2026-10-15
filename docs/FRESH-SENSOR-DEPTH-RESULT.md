# Fresh 005 Berkeley sensor-depth diagnostic

The [pre-score plan](FRESH-SENSOR-DEPTH-PLAN.md) was frozen before the one
approved live batch. The [per-ray report](/Volumes/backups/code/crispmesh-data/fresh-sensor-depth-005/report.json)
completed with SHA-256
`a4b3d0eda0c7fcb4f508e73f1ce30e5ebfab4112e0e75cc7ddec3524117a3105`
(1,153,432 bytes). Native rough and refined meshes came from the complete,
hash-sealed 005 producer. The evaluator exported their unchanged triangles to
bounded PLYs: 35,230 vertices/70,456 faces and 5,351 vertices/10,698 faces,
respectively; no zero-area triangles were excluded.

An independent post-reconstruction, named-camera-only fit bound the exact
fresh 004 sparse model to the sealed Berkeley table-frame camera metadata.
Its [camera report](/Volumes/backups/code/crispmesh-data/fresh-sensor-depth-005-preflight/camera-fit.json)
has SHA-256
`d9e51355eb7776132d8abba83d5d2b8be8ba2d6c073149b9d1d9d7a156efc3b6`.
All 60 names pair; center RMS is 9.24 mm, or 1.894% of camera radius, and
orientation p95 is 2.896°. The dense model retains the same named world poses
with maximum matrix difference zero. This fit uses Berkeley poses for
evaluation only. It is distinct from the earlier 008/014/015 camera gauges and
from the Google-mesh fit; no mesh residual was fitted.

All three selected-ray hashes and the 2,048 selected rays per view exactly
match `sensor-depth-004`, as do the observed depths and interior membership.
Eligible depth counts are 17,595 / 8,958 / 10,572, with 1,622 / 1,107 /
1,058 fixed eroded-interior rays. Thus the three earlier rough-stage rows
below are directly comparable under each arm's separately fitted camera-only
gauge. There is no compatible frozen sensor-depth score for cached native 011;
it is not ranked here. Refined 005 is a secondary within-run stage result,
not compared to a prior rough mesh as a refinement effect.

| Coarse support, pooled 0°/120°/240° | rough 008 | rough 014 | rough 015 | rough 005 | refined 005 |
| --- | ---: | ---: | ---: | ---: | ---: |
| Hits / 6,144 | 5,166 | 5,140 | 4,951 | 5,013 | 4,892 |
| Misses | 978 | 1,004 | 1,193 | 1,131 | 1,252 |
| Hit fraction | 84.08% | 83.66% | 80.58% | 81.59% | 79.62% |
| Hit-only mean absolute residual | 2.881 mm | 5.959 mm | 2.922 mm | 3.076 mm | 3.124 mm |
| ≤5 mm, all supported rays | 70.72% | 38.56% | 68.10% | 69.89% | 67.27% |

The new rough 005 pooled signed mean is +2.145 mm (predicted minus
observed); hit-only absolute median/p90/p95 are 2.221/6.193/8.958 mm.
Among its hits, 85.66%/96.23%/99.20% fall within 5/10/20 mm; among all
supported rays, the corresponding fractions are 69.89%/78.52%/80.94%.
For refined 005, signed mean is +2.368 mm; hit-only absolute
median/p90/p95 are 2.335/6.245/8.858 mm. Its 5/10/20 mm fractions are
84.48%/96.44%/99.24% among hits and 67.27%/76.79%/79.02% among all
supported rays.

| New arm / view | Hits / 2,048 | Misses | Hit-only mean absolute | Signed mean |
| --- | ---: | ---: | ---: | ---: |
| Rough 005 / 0° | 1,581 | 467 | 2.272 mm | +0.994 mm |
| Rough 005 / 120° | 1,767 | 281 | 3.829 mm | +2.909 mm |
| Rough 005 / 240° | 1,665 | 383 | 3.041 mm | +2.428 mm |
| Refined 005 / 0° | 1,549 | 499 | 2.233 mm | +1.037 mm |
| Refined 005 / 120° | 1,718 | 330 | 3.886 mm | +3.247 mm |
| Refined 005 / 240° | 1,625 | 423 | 3.168 mm | +2.707 mm |

On the fixed eroded-interior subset of 3,787 rays, rough 005 hits 3,479
(91.87%) with 2.363 mm hit-only absolute mean; refined 005 hits 3,462
(91.42%) with 2.400 mm. These are interior-support diagnostics, not
whole-object completeness.

The rough 005 arm has lower hit coverage and slightly larger hit-only
absolute mean than rough 008; it has better depth agreement than rough 014
on these fixed rays. The score does not establish which reconstructed shape
is physically more accurate. A camera-center fit residual of 9.24 mm,
Berkeley turntable-pose uncertainty, assumed IR depth scale and rectified
depth grid, RGB/depth reprojection, and photo-derived mask boundaries all
contribute to the observed residuals. Hits condition the residual statistics;
misses are kept separate. Both reference depth and poses stayed outside
reconstruction.

The live batch took about 6.2 seconds, below the 300-second cap. The JSON is
below 20 MiB; both disks retained over 10 GiB free (workspace 23.72 GB and
backup 17.13 GB at the end). The existing sensor-depth tests plus the fresh
gauge adapter test passed (12 tests).
