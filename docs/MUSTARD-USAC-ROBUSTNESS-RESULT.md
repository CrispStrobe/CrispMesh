# Mustard TRAIN two-view estimator robustness result

The approved batch followed the [frozen plan](MUSTARD-USAC-ROBUSTNESS-PLAN.md).
The [external JSON report](/Volumes/backups/code/crispmesh-data/mustard-usac-robustness-001.json)
completed in 1.272 seconds, is 1,139,483 bytes, and has SHA-256
`6128531963bfd236dbc1ddad1b83c5d7db266fd7feef11ae87bec8df14af2080`.
It used the pinned Python environment with OpenCV 4.10.0 and NumPy 1.26.4.
All six sealed TRAIN pairs in both arms received full-input fits in blob,
canonical, and reverse-canonical order. The stronger middle pair and both
opposing pairs also received three preselected leave-one-out removals per
arm in each order. Blob and canonical orders were identical on all rows;
reverse-canonical supplied the actual order perturbation.

The table gives the blob-order full-input status, an angle only when the
existing gates accepted a `distinct` or `near_identity` fit, the maximum
numerical rotation difference across full-input orders, and the maximum
same-order full-versus-removal rotation difference. Numerical spreads include
rotations from abstained fits and are diagnostics, not accepted poses.
`Abstain B; L` counts unavailable or indeterminate fits among the three
baseline orders and nine sampled removal/order fits. A dash under LOO means
that pair was outside the frozen removal panel. `USAC D/F/A` groups
`USAC_DEFAULT`, `USAC_FAST`, and `USAC_ACCURATE`: their complete recorded
method objects, including every per-seed count, angle, parallax, status,
homography count, order result, removal result, and summary, were exactly
equal on all 12 pair-arm rows. This is an observed equality on this panel,
not a general equivalence of the three algorithms.

| Arm | Pair | Method | Blob status | Accepted angle | Max order Δ | Max same-order LOO Δ | Abstain B; L |
| --- | --- | --- | --- | ---: | ---: | ---: | --- |
| cached | near 006/012 | RANSAC | unavailable | — | 42.09° | — | 2/3; — |
| cached | near 006/012 | USAC D/F/A | distinct | 30.93° | 51.80° | — | 0/3; — |
| cached | near 006/012 | MAGSAC | indeterminate | — | 9.46° | — | 3/3; — |
| cached | near 012/018 | RANSAC | unavailable | — | 0.96° | — | 3/3; — |
| cached | near 012/018 | USAC D/F/A | distinct | 63.18° | 4.36° | — | 0/3; — |
| cached | near 012/018 | MAGSAC | unavailable | — | 64.50° | — | 2/3; — |
| cached | middle 012/336 | RANSAC | distinct | 27.91° | 8.04° | 107.50° | 0/3; 0/9 |
| cached | middle 012/336 | USAC D/F/A | distinct | 43.22° | 31.41° | 41.58° | 0/3; 0/9 |
| cached | middle 012/336 | MAGSAC | unavailable | — | 0.00° | 143.46° | 3/3; 6/9 |
| cached | middle 036/126 | RANSAC | unavailable | — | 30.49° | — | 2/3; — |
| cached | middle 036/126 | USAC D/F/A | unavailable | — | 0.52° | — | 3/3; — |
| cached | middle 036/126 | MAGSAC | near_identity | 3.83° | 150.48° | — | 0/3; — |
| cached | opposing 030/198 | RANSAC | unavailable | — | 0.64° | 74.02° | 3/3; 6/9 |
| cached | opposing 030/198 | USAC D/F/A | unavailable | — | 45.43° | 136.53° | 2/3; 4/9 |
| cached | opposing 030/198 | MAGSAC | unavailable | — | 0.00° | 70.57° | 3/3; 5/9 |
| cached | opposing 036/222 | RANSAC | unavailable | — | 23.06° | 23.13° | 2/3; 9/9 |
| cached | opposing 036/222 | USAC D/F/A | unavailable | — | 0.00° | 0.67° | 3/3; 9/9 |
| cached | opposing 036/222 | MAGSAC | unavailable | — | 28.40° | 28.41° | 2/3; 7/9 |
| SAM | near 006/012 | RANSAC | unavailable | — | 42.09° | — | 2/3; — |
| SAM | near 006/012 | USAC D/F/A | distinct | 30.93° | 51.80° | — | 0/3; — |
| SAM | near 006/012 | MAGSAC | indeterminate | — | 9.46° | — | 3/3; — |
| SAM | near 012/018 | RANSAC | unavailable | — | 0.96° | — | 3/3; — |
| SAM | near 012/018 | USAC D/F/A | distinct | 63.18° | 4.36° | — | 0/3; — |
| SAM | near 012/018 | MAGSAC | unavailable | — | 64.50° | — | 2/3; — |
| SAM | middle 012/336 | RANSAC | distinct | 21.41° | 10.82° | 98.06° | 0/3; 0/9 |
| SAM | middle 012/336 | USAC D/F/A | distinct | 82.15° | 27.64° | 52.16° | 0/3; 0/9 |
| SAM | middle 012/336 | MAGSAC | unavailable | — | 0.01° | 112.92° | 3/3; 5/9 |
| SAM | middle 036/126 | RANSAC | unavailable | — | 30.49° | — | 2/3; — |
| SAM | middle 036/126 | USAC D/F/A | unavailable | — | 0.52° | — | 3/3; — |
| SAM | middle 036/126 | MAGSAC | near_identity | 3.83° | 150.48° | — | 0/3; — |
| SAM | opposing 030/198 | RANSAC | distinct | 89.10° | 158.13° | 125.48° | 0/3; 9/9 |
| SAM | opposing 030/198 | USAC D/F/A | unavailable | — | 0.64° | 0.38° | 3/3; 9/9 |
| SAM | opposing 030/198 | MAGSAC | distinct | 69.00° | 0.00° | 0.96° | 0/3; 0/9 |
| SAM | opposing 036/222 | RANSAC | unavailable | — | 23.06° | 23.13° | 2/3; 9/9 |
| SAM | opposing 036/222 | USAC D/F/A | unavailable | — | 0.00° | 0.67° | 3/3; 9/9 |
| SAM | opposing 036/222 | MAGSAC | unavailable | — | 28.40° | 28.41° | 2/3; 7/9 |

Across all 36 ordered baseline fits and 54 ordered removal fits per method,
the accepted-fit counts were RANSAC 15/36 and 21/54; each of the three
identical USAC D/F/A variants 19/36 and 23/54; and MAGSAC 13/36 and
24/54. Acceptance means the fixed numeric gates passed; it does not measure
physical accuracy. On SAM opposing 030/198, MAGSAC's accepted 69.00° fit
was stable under these sampled order/removal changes, whereas RANSAC's
accepted 89.10° full fit abstained on all nine sampled removals. MAGSAC
also had rotation differences above 140° on other sampled pairs. The
method comparison does not establish a generally robust estimator or a
true object rotation, and a larger perturbation sample could change the
stability ranking.

The runner SHA-256 is
`248fa086b09270741d3011bcab56f19e5595a65f75086e403d30807fc6569962`;
the historical pose and sealed-index loader hashes are respectively
`0ae09a5b5ea23b852a3166d42ab84ad0670d0255195f791e7cd2619bec135e05`
and `58379d118e90625481b746c503dc274414060490de89c8cc2a92cdf51c15aca6`.
Both source databases and producer results retained their pre-run SHA-256
hashes: cached DB/result
`5858be16dfa3f0fc50303d0eb4a001e21ee14036f67755a9997d5dac374b3075` /
`5a83c3e05e2b66887411799ecc8c5f3f41e3b8af3bbd775d308055ef7ee0041e`;
SAM DB/result
`09068fa4999a29fd9f61549ef4c56f3a92e7dbfc5cb2be62bf51ea9b72aee7d8` /
`4d6ce1bba0534c25035168d8c82565cf99ad2415c5c7327fb7ca6f0e6c4401d3`.
Both volumes stayed above 10 GiB free. No reconstructed camera, scanner,
depth, supplied pose, or held-out view entered a fit.
