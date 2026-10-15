# Mustard training-pose trajectory diagnostic

This is a read-only diagnostic of the 48 registered, image-estimated cameras
in the repaired mustard sparse model. The model was bound to the sealed
[`repair report`](../tests/evidence/mustard-sparse-repair-report.json)
(SHA-256 `32beea7955df0642cc87092afc39e96647adf9fbab582eaf14d6a6d662ef5ef8`)
and its three binary file hashes before and after reading. The full fresh
report is preserved byte-exact as
[`mustard-pose-trajectory.json`](../tests/evidence/mustard-pose-trajectory.json)
from the external artifact path
`/Volumes/backups/code/crispmesh-data/mustard-pose-trajectory-001.json`
(SHA-256 `082dee8e15473b6965bffa770a155acebae0992336a2a0855f358ce329c3fdbd`).
The frozen diagnostic runner is
[`pose_trajectory.py`](../scripts/object_motion/pose_trajectory.py)
(SHA-256 `bece9f5fd9d908b6922ce2e778b226202eebc10a030673f252713cc224b21144`);
three synthetic tests distinguish a circular orbit, a deliberate 180° fold,
and a displaced outlier.

Camera centers come from `-Rᵀt`, viewing directions from camera `+Z` in
world coordinates, and full orientation changes from relative rotations.
Reported center distances use the median radius of camera centers projected
onto their PCA plane **about the mean center**. This is descriptive
normalization, not a fitted circular orbit or metric scale. NP3 numeric
suffixes order the capture and define candidate 180-label pairings; they are
not supplied 3D poses or a physical turntable-angle calibration. The closing
`NP3_348→NP3_000` pair is listed separately with a wrapped label gap of 12.

| Read-only statistic | Recovered value |
| --- | ---: |
| PCA-plane residual / median planar radius, median / p95 / max | 0.142 / 0.329 / 0.436 |
| Adjacent center step / radius, median / p95 / max (47 pairs) | 0.175 / 2.597 / 2.946 |
| Adjacent full-orientation step, median / p95 / max | 6.49° / 81.69° / 92.32° |
| Opposing-label center distance / radius, median / p95 (24 pairs) | 0.179 / 2.671 |
| Opposing-label full-orientation angle, median / p95 | 3.86° / 86.75° |
| Closing center step / radius; full orientation step | 0.350; 13.44° |

Several opposing-label pairs are near-coincident in **both** center and full
orientation: `NP3_000/180` is 0.138 radius and 2.67°, `NP3_060/240` is
0.033 and 1.46°, `NP3_090/270` is 0.039 and 2.20°, and `NP3_078/258` is
0.071 and 1.25°. These paired photos can look visibly different despite
similar estimated poses. Conversely,
adjacent-label `NP3_270→276` jumps 2.946 radii and 92.32°, and
`NP3_096→102` jumps 2.834 radii and 85.17°. These center-plus-rotation
patterns support a **possible folded/aliased estimated trajectory**, not a
proof of physical camera motion or object-shape correctness. Some opposing
pairs are far apart, so the effect is not a uniform global 180° fold.

No reference mesh, supplied pose, held-out image, or filename-angle constraint
was used to estimate or change cameras. This report does not accept/reject
the reconstruction or tune the model. Sparse registration and track integrity
can coexist with trajectory ambiguity; a later independent camera or image
evaluation is needed before physical pose or geometry claims.
