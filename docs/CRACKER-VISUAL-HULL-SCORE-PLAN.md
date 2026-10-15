# Sealed cracker visual-hull scanner score — frozen, not executed

This is a separate evaluation-only score for the already sealed 60-view
image-only visual hull. The [producer](../scripts/classical_backend/cracker_visual_hull.py)
and its `hull.ply` are immutable. The Google YCB scanner enters only this
scoring process, never camera estimation, carving, mask choice, or bounds.

## Exact inputs and prerequisite replay

The candidate `cracker-visual-hull-001` result/report/mesh SHA-256 values are
`6bbef03514e8ac85139307afff59eaa767f58ba84a04f8c209e344337c0e06a8`,
`b0e88ce667f34e6e569aace1b21987f0fc3291813666887816e3c1556b05f2af`,
and `050f230c49f17d5e54fb5b15d8a2ddf4878e250d9662a9ad94fdd9eabdbc2de4`.
The unchanged scanner PLY SHA-256 is
`6e0187aa961aef4fa21dfc753023a4be7e82924e6398dc0875a2b0d609363ed4`.
Freeze score-005 JSON SHA-256
`165b8d5169e8bdb917f008864b01c434f4e03455b6ffe688d8ae8a824b2847c9`
and stage-diagnostic-005 JSON SHA-256
`5f6b1a6bf769516d2b0ff211cc4eb6204828b364fe5d98b7279032b41b6e48e2`.
Both old native meshes and their geometry-only normalization hashes are
checked in the [score wrapper](../scripts/classical_backend/score_cracker_visual_hull.py).
All inputs are rehashed before and after scoring.

Use exactly `score-005.shared_gauge.matrix_used_output_to_reference` as the
primary, unchanged 006-transported gauge for hull and both old meshes. The
matrix includes an earlier scanner-fitted parent transform, so this is a
**conditional shared-gauge shape comparison**, not physical pose or scale.
Before scoring the hull, rerun `surface_metrics.compare` on `mesh.ply` (rough)
and `refined.ply` (refined), and reproduce the saved 0.5/1/2%-of-scanner-AABB
diagonal thresholds and all saved threshold, precision, recall, F values to
absolute tolerance 1e-12. A mismatch abstains before candidate scoring;
there is no adjustment. Also replay the scanner self-control.

Every comparison uses 2,048 area samples per direction, reference seed 2027
and candidate seed 2028, exact point-to-triangle distances, and the same
thresholds. Primary report gives precision, recall, F, directional p95,
topology, and at each threshold `1-precision` extra-surface fraction and
`1-recall` unsupported-reference-surface fraction. These fractions are not
physical mass. A separate candidate-fitted proper Sim(3) uses 1,024 samples
per mesh and seed 2026, then the same surface score; fitted-only improvement
is secondary. Do not pick an alignment from its score.

Recover the exact occupied 160³ cells by X-ray parity through the neutral
mesh's exposed voxel faces and require the recovered count to equal 55,815.
Report fixed-gauge hull/scanner signed-volume ratio only if both meshes are
watertight. Report the fraction of fixed-gauge occupied voxel centers outside
the scanner AABB as a lower-bound extra-volume **proxy**, not IoU or a mass
measurement. The scanner does not change the grid or mesh.

## Bounded execution proposal

Fresh external output only:
`/Volumes/backups/code/crispmesh-data/cracker-visual-hull-score-001`.
The supervisor fully rehashes inputs before creating the directory, then
supervises a revalidating worker. Bound worker to 270 seconds, whole stage to
300 seconds, sampled RSS to 2 GiB, all new output to 20 MiB, log to 2 MiB,
and both disk floors to 10 GiB with output allowance. No source mutation,
download, rerun, alternative threshold or second candidate is authorized.

Proposed command only after explicit code review and approval:

```sh
PYTHONDONTWRITEBYTECODE=1 .local-tools/colmap-sparse/venv/bin/python -m scripts.classical_backend.score_cracker_visual_hull --output /Volumes/backups/code/crispmesh-data/cracker-visual-hull-score-001
```

The [synthetic tests](../scripts/classical_backend/test_score_cracker_visual_hull.py)
cover exact input hashes, fail-before-output preflight, baseline replay,
frozen thresholds, winding-independent voxel parity (including a cavity),
and count mismatch. No real scanner score has been run as part of these tests.
