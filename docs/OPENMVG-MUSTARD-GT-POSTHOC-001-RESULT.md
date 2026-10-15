# OpenMVG mustard posthoc camera check: both photo-only arms fail

The evaluation-only receipt at
`/Volumes/backups/code/crispmesh-data/openmvg-mustard-gt-posthoc-001/receipt.json`
is SHA-256 `863472bc0af7e69d0b16602dd8ea05ed59ead1a106d076b1f8ad01f002a1643e`,
status `posthoc_46_view_diagnostic_only`. Its report at
`/Volumes/backups/code/crispmesh-data/openmvg-mustard-gt-posthoc-001/report.json`
is SHA-256 `e6a791a8e16deba1ac6856b8f4eb90df7ce6ec4e2a092a53eb4b955892e8d035`.
This independent audit was read-only; it did not rerun SfM, scoring, or a
converter. Reference metadata entered **only after** both photo-derived SfM
models and JSON pose exports had been sealed.

The two exports independently link views to poses by `id_pose`, not array
order. They contain the **same 46 posed TRAIN names** and omit `NP3_042.jpg`
and `NP3_048.jpg`. Neither gains registration coverage. For each arm, the
frozen evaluator fits one positive-scale, proper center Sim(3) to those 46
names; leave-one-out (LOO) predictions refit on 45 and score the omitted
name. Orientation uses the **same center-fit world rotation**, not a second
orientation fit. Berkeley translation units are not independently certified
as metres.

| Posthoc measure, 46 common cameras | `ADJUST_ALL` | intrinsic `NONE` |
| --- | ---: | ---: |
| Median / p95 fitted center error ÷ Berkeley median radius | 0.9944 / 1.0346 | 0.9972 / 1.0428 |
| Median / p95 LOO center error ÷ radius | 1.0323 / 1.0680 | 1.0238 / 1.0697 |
| Max LOO center error ÷ radius | 2.0554 (`NP3_180`) | 2.0607 (`NP3_198`) |
| Median / p95 same-Q orientation residual | 160.79° / 177.27° | 165.34° / 177.92° |
| Fitted center RMS, Berkeley pose units | 0.49967 | 0.49962 |

The common reference median radius is `0.5007645` Berkeley pose units.
Errors of roughly **one orbit radius** after the best allowed global fit are
poor center-path agreement under this frozen protocol. The arms are
practically indistinguishable on the center metrics, and fixing intrinsics
does not repair either the two missing poses or the reference mismatch.
The paired median per-frame `NONE − ADJUST_ALL` center delta is only
`+0.00134` radius (25/46 positive); the median LOO delta is `+0.00129`
radius (also 25/46 positive). These are diagnostic differences, not evidence
of a useful improvement. Orientation residuals are very large, but their
absolute interpretation is conditional on the stated NP3/NP5 transform and
OpenCV world-to-camera axis convention; the unresolved board-frame offset
and depth validation preclude a definitive physical-orientation verdict.

The centered cross-covariance singular values are
`[0.0081230, 0.0026305, 9.846×10⁻¹¹]` for `ADJUST_ALL` and
`[0.0061964, 0.0016110, 7.259×10⁻¹¹]` for `NONE`. The smallest/largest
ratios, `1.21×10⁻⁸` and `1.17×10⁻⁸`, expose an **effectively planar** fit
despite numerical rank three and determinants near +1. The third alignment
axis is weakly constrained; the LOO maxima are additional instability
warnings. No reflection, outlier deletion, per-view correction, or
reference-guided retuning was used.

For context only, the earlier [calibrated checkerboard reference check](MUSTARD-CHECKERBOARD-CALIBRATED-EVALUATION-RESULT.md)
used the same pinned Sim(3)/LOO metric and NP3/NP5 composition and reported
center-error median `0.0242` radius and orientation median `1.50°` on a
**different 39-name subset**. That is a capture-calibration-assisted,
moving-board partial-arc method, not board-free reconstruction; its Berkeley
median radius was `0.482628`, and its convention/board offset also lacks
independent physical validation. Its much lower score therefore gives
context that the metric can show trajectory agreement under this convention,
but it is **not a paired, same-camera-set accuracy win** over OpenMVG.

Read-only provenance checks matched all **10** receipt-inventoried
non-receipt artifacts (nine staged NP5 H5 files plus report) by size and
SHA-256. The staged files total **20,664 bytes**, exactly nine at 2,296
bytes each; 38 other required H5 files matched the sealed extraction receipt.
The cached 657,272,400-byte mustard archive, metadata receipt, both export
receipts/JSONs/source models, fixed-arm source receipt, and pinned shared
metric source all matched their receipt hashes. The report's preflight input
block matches the final receipt's input seals (its `status` properly remains
`read_only_preflight_no_gt_scoring`). The posthoc task recorded 9.318 s;
the complete output is 94,508 bytes. At audit time both disks remained
above 11 GiB free. This is a failed camera-path diagnostic, **not** a dense
object/mesh result or a commercial/App Store license clearance.
