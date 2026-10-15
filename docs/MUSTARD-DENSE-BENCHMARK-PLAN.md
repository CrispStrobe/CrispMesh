# Mustard dense geometry: frozen conditional shape-oracle protocol

This is a **pre-score plan**, not a dense reconstruction result. The current
mustard SfM experiments produce sparse models only. No candidate dense mesh has
been scored, and no reference geometry may enter SfM, mask generation, native
depth, fusion, meshing, or repair selection. The separate Google scanner mesh
is a shape oracle with **no verified registration** to the Berkeley NP3 RGB
camera frame; it is not certified physical ground truth.

## Sealed evaluation-only reference

The existing VPS acquisition retains the Google 16k mesh at
`/mnt/akademie_storage/crispmesh-data/ycb-expansion-001/006_mustard_bottle/reference/`.
Only its two exact small PLY files were copied to the fresh external-SSD
`/Volumes/backups/code/crispmesh-data/mustard-eval-reference-001/` evaluation directory (no archive,
held-out photo, pose, or model copied):

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| `google_original_ascii.ply` | 987,348 | `0630e10cc921a98627c3f9baf79fb0e1236824b6a28bad1340a4f92f250e31c8` |
| `google_geometry_f64.ply` | 409,827 | `b82268e4ea6c9ad9dbcf8e6fd35b4a3b3631af3ecfcd413722df07f87cf601b4` |

These sizes and hashes match the pinned
[mustard dataset anchor](../tests/datasets/ycb_mustard_bottle.json).
The binary reference has 8,194 vertices, 16,384 triangular faces, two
zero-area triangles, no polygon faces or repeated indices, and finite XYZ.
The positive-area triangle evaluator omits the two zero-area faces and must
report that count. Its bounds are `[-0.063937999,-0.056809001,-0.003153]` to
`[0.033259999,0.0098120002,0.18814801]` in reference-coordinate units;
the bounding-box diagonal is `0.22468174167724111`. The numeric units are
not independently certified as metres here.

The frozen 4,096-sample reference-against-itself control ran after this copy:
all three F1 values were 1.0, normal absolute-dot means were 1.0, and
the symmetric Chamfer-L1 mean was `2.55e-18` reference-coordinate units
(floating-point noise, not zero-by-assertion). The exact
[self-control report](../tests/evidence/mustard-google-reference-self-control.json)
is SHA-256 `e0e13b588e0e4bc85981131b8b3989f70b7c3daf08bf1d5cfe28aa0b0bc9d336`.
A separate in-memory known proper-Sim(3) transform of this same reference
passed the sampled-to-triangle evaluator **when supplied the true transform**:
F1 `[1,1,1]`, symmetric mean distance `3.20e-16` reference units. This checks
the scorer, not search recovery. A second, blind alignment control used the
unchanged 1,024-sample aligner (seed 2026) to recover that known transform
before scoring independent 4,096 samples: rotation error 0°, relative scale
error `4.44e-16`, held-out corresponding RMS `1.84e-14` reference diagonals,
and F1 `[1,1,1]`. Its
[frozen control record](../tests/evidence/mustard-google-blind-sim3-control.json)
(SHA-256 `11b6044efad4b53945480bb9fa1529e874b5e9f73825bb85a2767da9d2b2ce81`)
binds `align.py` SHA-256 `24f27a0e2ff8879e5635151f73340b4abebbc04276f115cce627fec515f37b64`
and `surface_metrics.py` SHA-256 `466541e870f51411db647f96e1f1060e7c574ededdb9d92ded81b4ce43bebb16`.
These are identical-geometry numerical controls, **not** evidence that the
same search will align a partial or geometrically different candidate.

## Candidate eligibility and registration

Score only a finished, hash-sealed **triangular dense mesh** produced from the
48 training RGB photographs and accepted training-only masks. A sparse point
cloud, rejected candidate model, unfinished native stage, or copy without
source-stage provenance is ineligible. Validate candidate file/hash,
vertex/face counts, finite coordinates, triangle indices, degenerates, and
producer camera/mask lineage before alignment. Preserve all candidate faces:
no crop, component deletion, hole filling, score-selected ROI, or resampling
of the output mesh. Initially there is only one repaired dense candidate;
do not invent a raw-versus-repaired dense comparison.

Because the Google scan has a separate coordinate frame, use the existing
fixed [`align.py`](../scripts/object_dataset/align.py) proper-Sim(3) method
once on the entire mesh with 1,024 deterministic area-weighted surface samples
per mesh (seed 2026). Freeze the resulting matrix, scale, exact source and
reference SHA-256, script SHA-256, and alignment objective **before** seeing
surface scores. Fitted scale is reference-derived; it is not an independent
metric-scale estimate. The existing registration audit found partial-overlap
and local-minimum failures, so run known-transform/self controls and visually
inspect the full reference/output overlay. A visibly inconsistent orientation,
scale, or support means **registration unvalidated**; retain its failed-fit
evidence but do not interpret its F-score as shape quality or tune/retry the
fit against the score. If a later dense variant shares the same SfM world
gauge, reuse this exact frozen matrix for a shared-gauge comparison and verify
the camera/model-gauge lineage first; an independently fitted score may be
shown only as a separate sensitivity diagnostic.

## Surface metrics, fixed before any candidate score

Use [`surface_metrics.py`](../scripts/object_dataset/surface_metrics.py) with
4,096 deterministic area-weighted query samples per mesh, seed 2027 for
reference and 2028 for output. The target is the actual piecewise-linear
triangles, not nearest sampled target points. These evaluation samples differ
from the alignment samples. First run the reference-against-itself control;
it checks numerics and sampling, not reconstruction quality. Freeze and hash
the metric script, candidate PLY, reference PLY, and transform before the
candidate run; verify them again afterward. One score run is bounded by the
existing 300-second evaluator cap, ≤4,096 samples/mesh and existing geometry
caps; store a small JSON report, not per-point arrays.

Thresholds are fixed at **0.5%, 1%, and 2% of the reference bounding-box
diagonal**: `0.0011234087083862056`, `0.002246817416772411`, and
`0.004493634833544822` reference-coordinate units. Report each threshold's
output-to-reference precision, reference-to-output recall, and harmonic F1,
with denominators. Also report both distance means, medians and p95s,
normalized symmetric Chamfer-L1 mean, orientation-insensitive absolute-normal
consistency, and both meshes' connected components and boundary/nonmanifold
edge counts. Keep accuracy and completeness separate: a one-elevation photo
orbit can miss underside/handle regions even if visible surfaces fit. Whole
reference recall does not prove every region was observable. Do not award a
method a better score merely for deleting difficult geometry; retain candidate
face/area/topology and directional coverage context.

Any result is a **reference-fitted whole-mesh shape-oracle diagnostic**, not
held-out rendering quality, scanner metrology, physical millimetre accuracy,
or an accepted object-quality gate. No threshold, fit, crop, sample seed, or
mask choice may change after seeing candidate results. An alternate
camera-derived sensor-depth lane would answer a different question and needs
its own predeclared calibration/pose validation; it cannot validate this
Google-mesh alignment by itself.
