# Mustard independent two-view pose audit: frozen pre-score plan

This diagnostic reads only the sealed 48-TRAIN-view feature and verified-match
databases from `mustard-sfm-masked-fixed-exhaustive-001` (cached baseline) and
`mustard-feature-mask-pair-001-sam` (new SAM control). It does not read the
reconstructed camera poses, scanner, depth, held-out views, supplied Berkeley
poses, or the SQLite `qvec/tvec` placeholders. The latter are identity/zero
defaults in the cached database and are not measured relative poses.

The six unordered filename pairs below were fixed from cyclic acquisition
order and the pre-score verified-match counts. Suffixes are order labels, not
calibrated physical angles. Each pair must exist in both databases with at
least 20 verified correspondences; absent support is reported as unavailable,
not replaced by another pair. The near pair is the two strongest common
neighbor edges. The middle panel has its strongest common separation-5 edge
and strongest common edge at separation 12–15. The opposing panel has the
two lexicographically first among the strongest common separation-16–24 edges
(minimum count 25). No reconstructed-rotation value selected these pairs.

| Stratum | Pair | Cyclic positions apart | Cached / SAM verified rows |
| --- | --- | ---: | ---: |
| Near | NP3_006 / NP3_012 | 1 | 136 / 136 |
| Near | NP3_012 / NP3_018 | 1 | 129 / 129 |
| Middle | NP3_012 / NP3_336 | 5 | 41 / 42 |
| Middle | NP3_036 / NP3_126 | 12 | 24 / 24 |
| Opposing | NP3_030 / NP3_198 | 23 | 25 / 26 |
| Opposing | NP3_036 / NP3_222 | 23 | 25 / 25 |

Open each database with SQLite `mode=ro&immutable=1` after checking that
WAL/SHM sidecars are absent or empty. Decode its verified uint32 feature-index
pairs and float32 keypoints with bounds and duplicate checks. Require the
single shared `SIMPLE_RADIAL` 1280×1024 camera to equal the fixed initial
`[f=1536,cx=640,cy=512,k=0]` exactly in both databases. Treat this as an
assumed intrinsic, not measured calibration. Normalize keypoints through
that pinhole camera; no model-camera pose or photo-pixel source is loaded.

For each pair independently, use OpenCV 4.10 `findEssentialMat` five-point
RANSAC on the verified correspondences, confidence 0.999, 2-pixel equivalent
normalized threshold, at most 2,000 iterations, RNG seeds 17, 23, and 31,
one CPU thread. Resolve each essential pose with `recoverPose` on only that
seed's RANSAC inliers. Preserve the RANSAC and cheirality masks separately.
Record essential inlier counts, cheirality counts, full rotation angles,
rotation differences across seeds, median inter-ray parallax angle among
cheirality-positive points, and competing 2-pixel homography support. This is
an angle between camera bearing rays after rotating into one frame, not a
measured triangulated depth. Translation scale remains unknown. Never derive
the measured rotation from the saved SfM cameras.

Reliability gate per seed: at least 20 input correspondences, 15 essential
inliers and 65% essential-inlier fraction; at least 12 cheirality-positive
inliers and 60% of essential inliers; median inter-ray parallax at least
1°. Across all three seeds, recovered rotations must agree within 5°.
If a homography has at least as many inliers as the essential model, flag
planar/panoramic ambiguity and withhold a physical-rotation conclusion even
if the numerical essential angle is stable. A reliable full rotation below
5° is classified `near_identity`; above 15° is `distinct`; 5–15° remains
`indeterminate`. Failed gates are `unavailable`, with counts and cause. The
classification concerns support under the fixed camera and rigid-scene
model; it cannot certify true object rotation or identify why SfM folded.

Run only after root reviews the source/panel/API preflight. One bounded batch
has a 30-second process alarm, at most 12 pair-arm fits (three RANSAC seeds
each), <20 MiB report, and a 10 GiB free-space floor on both workspace and
backup volumes. New output is external under
`/Volumes/backups/code/crispmesh-data`; no large `/tmp` artifacts. Hash the
input database and producer reports before and after. Do not change the panel,
seeds, threshold, or gates after observing a pose result.
