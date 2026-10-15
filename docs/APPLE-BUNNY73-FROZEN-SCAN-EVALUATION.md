# Apple bunny `.preview`: frozen scan diagnostics

The Apple `.preview` mesh was evaluated with the same frozen whole-scanner and
scanner-only ROI protocol used for the existing MVE/COLMAP bunny diagnostics.
The [Apple USDZ](/Volumes/backups/code/crispmesh-data/apple-bunny73-preview-001-output/model.usdz)
and its [untextured evaluation PLY](/Volumes/backups/code/crispmesh-data/apple-bunny73-review-001/mesh-untextured.ply)
were read only. Their SHA-256 hashes, the full scan, the ROI, the previous
receipts, and the unchanged evaluation scripts passed preflight. Apple Object
Capture is macOS-only; `.preview` is its coarse detail setting. No scanner
geometry or supplied poses entered reconstruction.

For each reference independently, the unchanged aligner fitted a proper
Sim(3) from 1,024 area-weighted samples, seed 2026. The unchanged triangle
distance scorer used 4,096 samples per direction, seed 2027. The full scan
used absolute thresholds 1.0429398885, 2.085879777, and 4.171759554 in
scanner-coordinate units. The scanner-only ROI used 0.6140926143,
1.2281852285, and 2.4563704570 (0.5/1/2% of its own bbox diagonal), plus
the same three original absolute thresholds in one draw. Every Apple triangle
was retained in both panels; the scanner-only ROI retains triangles only when
all vertices have Y > -24.

The bounded [run receipt](../build-opencv/apple-bunny73-evaluation-001/result.json)
reports all four stages complete in 52.5 seconds of stage time and 40,098
bytes of output, well below the 8 MiB cap. Both disk floors remained above
11 GiB. The [whole fit](../build-opencv/apple-bunny73-evaluation-001/whole-reference-fit.json)
and [ROI fit](../build-opencv/apple-bunny73-evaluation-001/roi-reference-fit.json)
are independent fits to their respective reference surfaces. Their fitted
scales, 291.20 and 207.31, are **reference-derived**, not measured physical
scale.

The [whole-scan report](../build-opencv/apple-bunny73-evaluation-001/whole-surface-metrics.json)
gives F = 8.48%, 16.41%, and 32.09% at the three frozen thresholds. At 1%,
precision is 18.33% and recall is 14.84%; normalized Chamfer-L1 mean is
4.22% of the full scanner bbox diagonal. The [whole-scan fitted preview](.local-tools/apple-bunny73-whole-fit-preview.png)
shows an upright scanner bunny on its broad disk beside a sideways fitted
Apple bunny. This orientation/support disagreement makes the numbers poor
evidence of intrinsic mesh quality. The native Apple preview itself has a
recognizable bunny silhouette.

The [ROI report](../build-opencv/apple-bunny73-evaluation-001/roi-surface-metrics.json)
gives, at the ROI's 1% threshold, precision 28.52%, recall 30.96%, and
F = 29.69%. At the original absolute 2.085879777-unit threshold, precision
is 44.09%, recall 45.09%, and F = 44.59%. ROI-normalized Chamfer-L1 mean is
2.83%. The [ROI fitted preview](.local-tools/apple-bunny73-roi-fit-preview.png)
shows a broadly bunny-like XY outline, but the ears, body, and other
projections still differ. Cropping the reference removes much of its disk and
changes the fitted pose and scale. The ROI is a post hoc development
diagnostic, not held-out object ground truth.

These F values are conditional on separate scanner-fitted poses. The existing
[registration audit](BUNNY-CLASSICAL-COMPARISON.md) found that this
cross-sensor alignment is unreliable for deciding which reconstruction has
better shape. Numerical differences from MVE or COLMAP/OpenMVS therefore do
**not** establish a pipeline ranking, KIRI-level performance, metric accuracy,
or product readiness. The whole and ROI panels answer different tolerance
questions and must not be combined into one score. Visual silhouette,
thin-feature coverage, support/background inclusion, and texture quality
remain separate observations; the [USDZ review](APPLE-BUNNY73-PREVIEW-REVIEW.md)
documents those structural checks.

Reproduce the read-only preflight with:

```sh
python3 -m scripts.apple_object_capture.evaluate_bunny_preview
```

The `--execute` option requires a fresh default output directory, so an exact
rerun needs a fresh path in the script or a separate checkout. It never
modifies the Apple USDZ, its PLY export, or scanner references.
