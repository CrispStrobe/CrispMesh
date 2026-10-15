# Proposed Apple mustard whole-mesh scanner diagnostic — not executed

The Apple `.preview` candidate is sealed and structurally valid, but its
[neutral preview](MUSTARD-APPLE-PREVIEW-RESULT.md) visibly includes a broad
support surface around a much smaller upright form. This proposal is a
separate evaluation-only experiment. It is **not authorized by the Apple
reconstruction run** and has not been executed. The scanner mesh may enter
only this post-candidate scoring lane, never Apple inference, component
selection, cropping, fit tuning, or reruns.

Use the exact untextured full Apple PLY at
`/Volumes/backups/code/crispmesh-data/apple-mustard48-review-001/mesh-untextured.ply`,
SHA-256 `c460f11fea76f9f2a64e8d60a9ec3ddcc6241f503aedc89745ab1c8a6b430004`
(935 vertices, 1,847 triangles). Keep every face, including the visible
support surface. Use the sealed Google 16k mustard geometry at
`/Volumes/backups/code/crispmesh-data/mustard-eval-reference-001/google_geometry_f64.ply`,
SHA-256 `b82268e4ea6c9ad9dbcf8e6fd35b4a3b3631af3ecfcd413722df07f87cf601b4`
(8,194 vertices, 16,384 triangles, two zero-area faces). The scanner's
bounding-box diagonal is `0.22468174167724111` in reference-coordinate
units. These units are not independently certified physical metres.

Before any fit, rehash both PLYs, validate them with unchanged `evaluate.py`
(SHA-256 `33f474f1a73c70dbc529ed746d5b8d98de42948687013df30eed22ec3912c89a`),
and confirm the frozen same-reference control report SHA-256
`e0e13b588e0e4bc85981131b8b3989f70b7c3daf08bf1d5cfe28aa0b0bc9d336`.
That 4,096-sample, seed-2027/2028 control has F1 1.0 at all three thresholds
and symmetric Chamfer-L1 `2.55e-18`; it checks numerical machinery only.
The prior known-transform and blind alignment self-controls are documented in
`MUSTARD-DENSE-BENCHMARK-PLAN.md`. They do not validate an alignment between
this board-dominated Apple mesh and the separate scanner object.

If separately approved, run unchanged `align.py` SHA-256
`24f27a0e2ff8879e5635151f73340b4abebbc04276f115cce627fec515f37b64`
once on the entire meshes: proper Sim(3), 1,024 deterministic area-weighted
samples per mesh, seed 2026, 60-second internal deadline. Seal its transform,
fit objective, fitted scale, script/input hashes, and convergence diagnostics
before surface scoring. The fitted scale is reference-derived. Use existing
paired and shared-bounds overlay previews for human registration review;
the overlay code SHA-256 is
`d3f459bc3e8381a450ea18e4ecca23810be4b3259a501906cdf006f6545d0409`.
The support-surface leakage must remain visible in these previews. A fit that
places the broad sheet over the bottle or lacks plausible full-mesh overlap
is **registration unvalidated**; retain its evidence and do not interpret
or tune a surface score. Do not try alternate initial seeds, remove the
support, or select a transform by F-score.

Only if the frozen transform passes that visual review, run unchanged
`surface_metrics.py` SHA-256
`466541e870f51411db647f96e1f1060e7c574ededdb9d92ded81b4ce43bebb16`
once, with 4,096 area-weighted queries per mesh, reference seed 2027 and
Apple output seed 2028, against exact target triangles. The scorer's deadline
is 300 seconds. Use exactly 0.5%, 1%, and 2% of the reference bounding-box
diagonal as thresholds: `0.0011234087083862056`,
`0.002246817416772411`, `0.004493634833544822`. Report output-to-reference
precision, reference-to-output recall, F1, distance mean/median/p95,
normalized symmetric Chamfer-L1, absolute-normal agreement, boundary and
nonmanifold edges, components, and the visible support-surface context. Retain
all producer costs and the visibly leaked sheet; no score-selected cleanup.
Rehash inputs, transform, and scripts after scoring. Use one fresh small
evaluation directory and preserve failures.

Even with a plausible fit, these would be reference-fitted whole-mesh shape
diagnostics, not independent metric accuracy or a physical-scale result.
The scanner has no verified transform into the Berkeley photo frame. A poor
whole-mesh score may reflect the visible board leakage and alignment
difficulty; a good score cannot establish held-out rendering or clean object
isolation. Compare another method numerically only under the same reference,
fit policy, sample counts, thresholds, and full-mesh scope.
