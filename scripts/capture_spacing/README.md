# Photo spacing while taking photos: tried, not shipped

The question: can *Take photos* warn while capturing that two consecutive photos are too far apart for the camera
solver, with a measure cheap enough for a browser (well under 200 ms on small frames)? The outcomes it would have
to predict (photos stage, 3DLF objects, `--minimum-observations-per-view 8`):

| Step | Photos | Bunny | Armadillo | Dragon | Lucy | Thai statue | Happy Buddha |
|---|---|---|---|---|---|---|---|
| every 2nd (10°) | 37 | pass | pass | pass | pass | pass | pass |
| every 4th (20°) | 19 | pass | pass | refused | refused | pass | turn not found |
| every 5th (25°) | 15 | refused | | | | | |

`pair_measures.py TEST_DATA OUT.json` computes, for all 73 pairs of a turn and steps of 1 to 5 photos, SIFT
matches at full size on the object (close to the solver's features), ORB matches at 320 / 480 / 640 pixels with the
static backdrop excluded, and an apparent turn from the matches' horizontal motion. Fundamental-matrix inliers,
median / 10th percentile / minimum over the 73 pairs:

| SIFT inliers | 1 (5°) | 2 (10°) | 3 (15°) | 4 (20°) | 5 (25°) |
|---|---|---|---|---|---|
| Bunny | 17 / 14 / 12 | 12 / 10 / 8 | 11 / 9 / 7 | 10 / 8 / 0 | 10 / 8 / 0 |
| Armadillo | 72 / 56 / 50 | 39 / 26 / 17 | 23 / 16 / 14 | 17 / 13 / 11 | 14 / 12 / 10 |
| Dragon | 70 / 31 / 19 | 40 / 15 / 11 | 24 / 12 / 9 | 16 / 10 / 9 | 14 / 11 / 8 |
| Lucy | 45 / 32 / 25 | 30 / 19 / 15 | 21 / 14 / 11 | 17 / 12 / 10 | 14 / 11 / 9 |
| Thai statue | 104 / 81 / 52 | 59 / 46 / 35 | 34 / 25 / 15 | 20 / 14 / 12 | 16 / 13 / 11 |
| Happy Buddha | 53 / 40 / 27 | 31 / 23 / 17 | 20 / 17 / 13 | 18 / 13 / 11 | 16 / 12 / 11 |

| ORB inliers, 640 px | 1 | 2 | 3 | 4 | 5 |
|---|---|---|---|---|---|
| Bunny | 112 / 83 / 0 | 68 / 46 / 39 | 45 / 30 / 25 | 32 / 23 / 19 | 28 / 19 / 17 |
| Armadillo | 231 / 182 / 161 | 135 / 115 / 87 | 85 / 64 / 52 | 55 / 43 / 34 | 43 / 32 / 28 |
| Dragon | 294 / 153 / 102 | 179 / 62 / 35 | 116 / 31 / 25 | 70 / 23 / 16 | 50 / 19 / 16 |
| Lucy | 196 / 140 / 100 | 133 / 89 / 58 | 88 / 60 / 43 | 68 / 50 / 35 | 52 / 41 / 34 |
| Thai statue | 261 / 229 / 190 | 177 / 151 / 130 | 107 / 80 / 58 | 64 / 47 / 39 | 46 / 35 / 27 |
| Happy Buddha | 209 / 149 / 108 | 146 / 83 / 72 | 104 / 65 / 47 | 83 / 52 / 39 | 69 / 46 / 29 |

(ORB at 320 and 480 pixels ranks the objects the same way; ORB inliers vary by a few between runs, RANSAC is
random.) Apparent turn, median degrees (true: 4.9° per photo):

| | 1 | 2 | 3 | 4 | 5 |
|---|---|---|---|---|---|
| Bunny | 2.8 | 4.4 | 6.0 | 7.5 | 8.8 |
| Armadillo | 2.5 | 3.1 | 4.1 | 5.8 | 7.1 |
| Dragon | 2.7 | 4.9 | 7.3 | 9.9 | 11.9 |
| Lucy | 2.0 | 2.7 | 3.5 | 4.1 | 4.8 |
| Thai statue | 1.8 | 2.1 | 2.5 | 3.1 | 3.5 |
| Happy Buddha | 3.0 | 2.8 | 3.3 | 3.9 | 4.4 |

None separates. The Bunny, which passes at every 4th photo, has the fewest matches of all six at every step
(fewer at 20° than the Dragon, Lucy and Happy Buddha, which fail); no threshold passes the Bunny at 20° and refuses
the Dragon at 20° without also refusing the Bunny at 10°. The solver's own counts at 19 photos agree: observations
Bunny 456 and Armadillo 499 (pass), Dragon 415 and Lucy 547 (refused), Thai statue 3 237. The failures are
geometric (camera centres off the ring plane, a 35° gap, a turn found as 182°), not a shortage of matches between
neighbours that a pair measure could see. The apparent turn is no better: cross-checked matches survive only where
the motion is small, so it reads 3° to 10° for a true 20° step and orders the objects by shape, not by step.
Nothing was shipped; the photo-count choice in *Take photos* (36 for every object, 20 with the neural surface)
stays the guidance.
