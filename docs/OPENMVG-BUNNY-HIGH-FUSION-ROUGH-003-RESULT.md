# OpenMVG HIGH bunny cached-fusion rough result

The 73-map, `--geometric-iters 0` OpenMVS arm produced a native rough mesh,
but **failed the independent object-shape gate**. No refinement, texture,
scanner-fit score, or pipeline ranking was run from this mesh.

The frozen native root is
`/Volumes/backups/code/crispmesh-data/openmvg-bunny-high-openmvs-cache-fusion-003`.
The original `result.json` (SHA-256
`bb7b7d928b245931afe0c7646bee73bf7fe2fe752b399a347687a96fd2955fc2`)
records `failed` solely because the runner expected `mesh.mvs` after
`ReconstructMesh`. Its own native stage records show both `densify` and
`mesh` completed with return code 0. The exact `ReconstructMesh` log says it
saved `mesh.ply`; no `mesh.mvs` was emitted. This is a final receipt-contract
error, not a native reconstruction crash. The failed receipt and all native
outputs remain unchanged. The runner's future artifact contract now expects
only `dense.mvs`, `dense.ply`, and `mesh.ply`.

A separate [post-hoc rescue validator](../scripts/classical_backend/openmvg_bunny_high_rough_rescue.py)
sealed the failed receipt, both stage logs, the three actual artifacts, all
73 cached base maps and images, original source lineage, resource caps, and
native geometry. Its sidecar is
`.local-tools/openmvg-bunny-high-fusion-rough-rescue-003.json`
(SHA-256 `6f32778a79c2f2f911fb4c8745e812baebe299b963dd07b3c11392bc221dd42c`),
with status `native_rough_complete_pending_visual_review`; it is **not** a
positive quality approval. Native `dense.ply` has 569,064 points. Native
`mesh.ply` (SHA-256
`99dd17324be3c6a5360de4f95512ba6531de8424050ee950f845f8695a83416d`)
has 178,767 vertices, 357,469 triangle faces, and zero zero-area faces.
Its geometry contains six vertex-connected components, with 354,555 faces
in the largest and 1,308, 1,202, 382, 18, and 4 in the others. The run
consumed 152.808 real-wall seconds, 903,900,892 logical output bytes, and
48,435,200 incremental physical bytes; `source_unchanged` is true and the
11 GiB reserve remained intact.

The lossless neutral geometry export is
`.local-tools/openmvg-bunny-high-fusion-rough-neutral-003.ply`
(SHA-256 `78daf68ba049bc1b089c65696ae71e72f06adcd430befa9b3f65cfb31ea45970`).
The [candidate-only XY/XZ/YZ preview](../.local-tools/openmvg-bunny-high-fusion-rough-neutral-003.png)
(SHA-256 `1361688445c0e55b9534afd6394241990a8caa47c108d2075c5c8331da20fb3f`)
uses 10,000 deterministic area-weighted samples and common candidate bounds,
without scanner alignment. It shows fragmented form and broad support or
background surfaces, not a coherent bunny silhouette. Component counts do
not rescue that visual defect: most faces are connected in one component.
The separate [review receipt](../.local-tools/openmvg-bunny-high-fusion-rough-visual-review-003.json)
records `rejected_for_refine_texture`, bound to the failed and rescue receipt
hashes and exact mesh. No scanner-fit F score was needed to reject this
object-shape result, and none should be interpreted as an intrinsic ranking.
