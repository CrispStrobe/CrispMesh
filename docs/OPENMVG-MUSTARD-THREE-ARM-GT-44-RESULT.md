# OpenMVG mustard three-arm posthoc check: common 44 remain poor

The read-only-audited receipt at
`/Volumes/backups/code/crispmesh-data/openmvg-mustard-gt-posthoc-three-arm-001/receipt.json`
is SHA-256 `576b5638985a745d3718dfa59c616a8b2b8ad8045a4470d61bcfd66ca080388e`;
its report at
`/Volumes/backups/code/crispmesh-data/openmvg-mustard-gt-posthoc-three-arm-001/report.json`
is SHA-256 `992124c1f9066539c3ad6ad8ac76b51eb696bf1af536cf9d0ce0702d193d734b`.
The receipt status is `posthoc_common_44_diagnostic_only`, not camera
acceptance. This independent audit did not rerun scoring or modify any
producer/reference artifact.

Directly checking the three sealed JSON exports' view `id_pose` links confirms
each model has **46/48** TRAIN poses. Incremental `ADJUST_ALL` and intrinsic
`NONE` both lack `NP3_042` and `NP3_048`; GLOBAL `ADJUST_ALL` lacks
`NP3_252` and `NP3_276`. Their exact-name intersection is the report's **44
TRAIN cameras**, excluding all four. Each arm receives a new proper,
positive-scale center Sim(3) on those same 44 names; the previous two-arm
46-name fit errors are **not** reused. All paired per-frame deltas in the
report equal the differences of its 44 per-frame rows. Berkeley calibration
and poses were posthoc inputs only, not supplied to any SfM arm.

| Common-44 measure | Incremental `ADJUST_ALL` | Incremental `NONE` | GLOBAL `ADJUST_ALL` |
| --- | ---: | ---: | ---: |
| Fitted center error / reference median radius, median | 1.0007 | 1.0038 | 0.9786 |
| Fitted center error / radius, p95 | 1.0331 | 1.0351 | 1.1478 |
| Leave-one-out center error / radius, median | 1.0293 | 1.0305 | 1.0054 |
| Leave-one-out center error / radius, p95 | 1.0702 | 1.0690 | 1.2434 |
| Leave-one-out center error / radius, max | 2.0877 | 2.0775 | 1.6221 |
| Same-center-fit-Q orientation residual, median | 159.86° | 164.66° | 128.52° |
| Same-Q orientation residual, p95 | 177.28° | 177.29° | 174.15° |

The common Berkeley median center radius is `0.50004194` in its native
pose-translation units, not independently established metres. All three
best-fit center paths miss by about **one radius**, an unconvincing camera
recovery result. GLOBAL's modestly lower median is accompanied by a worse
center/leave-one-out tail, so it is not a clear improvement. Its lower
orientation median is still about **129°**, not plausible alignment. The
orientation comparison depends on the documented NP3/NP5 matrix and OpenCV
world-to-camera conventions; `/board_frame_offset` and depth have not
independently validated the absolute convention. This caveat does not erase
the large center residuals under the frozen comparison.

The centered cross-covariance smallest/largest singular-value ratios are
`1.03×10⁻⁸` (incremental `ADJUST_ALL`), `1.11×10⁻⁸` (`NONE`) and
`1.91×10⁻⁸` (GLOBAL). Although each numerical rank is three and fitted
rotation determinant is +1, all alignments are **effectively planar** and
weakly constrain the third axis. Leave-one-out maxima occur at `NP3_180`
(2.0877), `NP3_198` (2.0775), and `NP3_336` (1.6221), respectively.
No reflection, per-view correction, outlier deletion, or GT-guided retuning
is authorized by these scores.

The report is the only non-receipt output: its 101,015 bytes and hash match
the receipt inventory; the two-file tree totals 117,663 bytes. The 45 used
H5 files (44 NP5 poses plus calibration) rehash to their receipt records, as
do the three JSON exports and their receipts/models, the previous two-arm
receipt/report, and the pinned shared evaluator source SHA-256
`f24442884ecb9e102ee5ad96fb4727b56dc30e94d0f92aa783c6350704878c7b`.
The report's input block matches the final receipt except for its expected
preflight-only `status`. No methodological population/delta mismatch was
found in this read-only check. The 2.617 s scorer run stayed within its
recorded cap; both disks were above 11 GiB at audit time. This remains an
evaluation-oracle comparison, not validated object geometry or a
commercial/App Store license clearance.
