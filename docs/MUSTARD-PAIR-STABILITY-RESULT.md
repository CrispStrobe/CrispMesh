# Mustard TRAIN verified-correspondence stability result

The approved one-batch run followed the [frozen perturbation plan](MUSTARD-PAIR-STABILITY-PLAN.md)
on the [sealed six-pair panel](MUSTARD-TWO-VIEW-POSE-PLAN.md). The
[external JSON report](/Volumes/backups/code/crispmesh-data/mustard-pair-stability-001.json)
completed in 1.080 seconds, contains 184 leave-one-out removals and two
single verified additions, and is 526,064 bytes with SHA-256
`50fee1903d0bb0f855b2fb9acf0568155c20209f0a68fdb1a797993359b8dde5`.
The stability runner SHA-256 is
`58379d118e90625481b746c503dc274414060490de89c8cc2a92cdf51c15aca6`;
the unchanged pose runner is
`0ae09a5b5ea23b852a3166d42ab84ad0670d0255195f791e7cd2619bec135e05`.

| Pair / arm | Full-input status and angle | Leave-one-out status counts | Maximum rotation difference from full input | E inliers / cheirality / median ray parallax across removal fits |
| --- | --- | --- | ---: | --- |
| Middle 012/336 cached | distinct, 27.91° | 41 distinct | 107.50° | 34–36 / 27–36 / 1.36–109.17° |
| Middle 012/336 SAM | distinct, 21.41° | 38 distinct, 4 unavailable | 137.70° | 33–36 / 25–36 / 0.90–137.79° |
| Opposing 030/198 cached | unavailable, numerical 1.68° | 18 unavailable, 5 distinct, 2 near-identity | 55.96° | 20–21 / 5–21 / 0.10–56.20° |
| Opposing 030/198 SAM | distinct, 89.10° | 23 unavailable, 1 distinct, 2 near-identity | 146.03° | 20–22 / 4–20 / 0.26–62.47° |
| Opposing 036/222 cached | unavailable, numerical 2.01° | 25 unavailable | 4.24° | 21–23 / 5–21 / 0.08–1.00° |
| Opposing 036/222 SAM | unavailable, numerical 2.01° | 25 unavailable | 4.24° | 21–23 / 5–21 / 0.08–1.00° |

The counts above are per leave-one-out trial. The inlier, cheirality, and
parallax ranges include all three fixed seeds; a numerical angle on an
`unavailable` fit is not a physical-rotation conclusion. In all trials with
recoverable rotations, interseed numerical spread stayed below
0.000002°. This repeatability does not establish stability to a changed
correspondence set: 95 of 184 removal trials abstained, and the accepted
angles can move by more than 100°.

The extra SAM middle match `(258,228)` is last in its verified blob. Adding it
to the cached row reproduces the SAM middle 21.41° result; removing it from
SAM reproduces the cached middle 27.91° result. The extra SAM opposing match
`(232,26)` is at row 21, while this frozen addition protocol appends it to
the cached row. Appended to cached, that 26-point set remains unavailable
(1.56° numerical angle, 21 essential inliers, 15 cheirality-positive,
0.20° parallax). In its SAM order, the same feature-index set gives the
89.10° distinct baseline. Removing row 21 from SAM restores the cached
25-point unavailable result (1.68°, 22 essential inliers, 19
cheirality-positive, 0.27° parallax). All selected-pair decoded keypoint
coordinates are exactly equal across arms, and common verified matches retain their order.
Thus correspondence order also affects this seeded RANSAC estimate; the
single-match difference alone cannot be assigned the full 89° shift.

The immutable source databases and producer reports retained their exact
SHA-256 hashes after the run: cached DB/result
`5858be16dfa3f0fc50303d0eb4a001e21ee14036f67755a9997d5dac374b3075` /
`5a83c3e05e2b66887411799ecc8c5f3f41e3b8af3bbd775d308055ef7ee0041e`,
SAM DB/result
`09068fa4999a29fd9f61549ef4c56f3a92e7dbfc5cb2be62bf51ea9b72aee7d8` /
`4d6ce1bba0534c25035168d8c82565cf99ad2415c5c7327fb7ca6f0e6c4401d3`.
Both volumes remained above 10 GiB free. No scanner, depth, held-out view,
or reconstructed camera pose entered any fit. The opposing-view result does
not independently establish physical rotation, and the middle-view angle
magnitude is likewise fragile under one verified-match removal.
