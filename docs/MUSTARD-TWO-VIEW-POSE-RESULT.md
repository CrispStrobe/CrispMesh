# Mustard TRAIN-only independent two-view pose result

The one approved batch used the [frozen six-pair plan](MUSTARD-TWO-VIEW-POSE-PLAN.md).
Its [external report](/Volumes/backups/code/crispmesh-data/mustard-two-view-pose-001.json)
completed in 0.065 seconds, is 45,337 bytes, and has SHA-256
`c4c17d85d8c61f06201009bbe8d59fe0d15d5bd8c9b63169890b47f8531beece`.
The fixed runner SHA-256 is
`0ae09a5b5ea23b852a3166d42ab84ad0670d0255195f791e7cd2619bec135e05`.
Both input databases and producer reports retained their pinned hashes after
the read-only fit: cached DB/result
`5858be16dfa3f0fc50303d0eb4a001e21ee14036f67755a9997d5dac374b3075` /
`5a83c3e05e2b66887411799ecc8c5f3f41e3b8af3bbd775d308055ef7ee0041e`;
SAM DB/result
`09068fa4999a29fd9f61549ef4c56f3a92e7dbfc5cb2be62bf51ea9b72aee7d8` /
`4d6ce1bba0534c25035168d8c82565cf99ad2415c5c7327fb7ca6f0e6c4401d3`.
Both disks remained above 10 GiB free (workspace 23.68 GB, backup 17.01 GB).

| TRAIN pair | Cached: E / cheirality / H, parallax, result | SAM: E / cheirality / H, parallax, result |
| --- | --- | --- |
| Near 006/012 | 131 / 77 / 115, 0.10°, unavailable | same |
| Near 012/018 | 123 / 58 / 102, 0.11°, unavailable | same |
| Middle 012/336 | 34 / 34 / 29, 2.93°, distinct 27.91° | 36 / 36 / 28, 1.69°, distinct 21.41° |
| Middle 036/126 | 23 / 22 / 17, 0.41°, unavailable | same |
| Opposing 030/198 | 22 / 19 / 14, 0.27°, unavailable | 21 / 21 / 15, 88.61°, distinct 89.10° |
| Opposing 036/222 | 24 / 13 / 17, 0.17°, unavailable | same |

The table shows one seed's counts; all three frozen seeds produced the same
estimate within numerical precision for these inputs. `E` is essential
RANSAC inliers, `H` is competing homography inliers, and parallax is the median angle between rotated bearing
rays among cheirality-positive points. It is not a measured triangulated
depth. `Unavailable` means at least one preset reliability gate failed; for
both near rows, middle 036/126, and three of the four opposing pair-arm rows,
the median parallax was below 1°. Near 006/012 also has only 77/131
cheirality-positive E inliers (58.8%) and near 012/018 only 58/123 (47.2%);
opposing 036/222 has 13/24 (54.2%). Each falls below the fixed 60%
cheirality fraction gate. These angles must not be read as reliable
near-identity pose measurements. The original database configurations for
these pairs were `PLANAR_OR_PANORAMIC` (6) or `UNCALIBRATED` (3); neither
label supplies an independent pose.

The apparent SAM `030/198` opposing-pair 89.10° result passes the predeclared
numeric gates, but a read-only exact feature-index comparison found its 26
verified correspondences contain all 25 cached correspondences plus just one
new pair, indices `(232,26)`. That single addition coincides with the
essential solution changing from a low-parallax 1.68° fit to 89.10°.
The frozen protocol does not permit dropping the point or choosing another
fit. This correspondence sensitivity makes the 89.10° result weak evidence
of physical rotation despite its within-database three-seed stability. It
does not verify the physical rotation implied by the acquisition separation.
The middle `012/336` SAM row likewise adds one verified match to the cached
41, and both middle fits pass the declared gates at different angles.

Thus the selected long-separation verified matches do not independently
establish either a physically distinct opposing-view rotation or a true
near-identity collapse. One long SAM essential estimate supports a distinct
rotation under fixed assumed intrinsics and the rigid-scene model, while the
other three long pair-arm estimates are unavailable. Repeated texture,
planarity, low parallax, and an uncalibrated fixed initial focal length can
make an essential solution misleading. The database `qvec/tvec` defaults and
final reconstructed camera poses were not used as measured relative poses;
no scanner, depth, held-out images, or supplied Berkeley poses entered the
fit. The diagnostic neither validates physical camera orientation nor proves
why the 48-view SfM trajectory folded.
