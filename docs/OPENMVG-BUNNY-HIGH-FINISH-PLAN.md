# Gated OpenMVG HIGH bunny rough-to-texture continuation

Status: version-3 rescue-sidecar adapter prepared, synthetic-tested, **not run**. This is a separate composed continuation of the 73-photo OpenMVG HIGH sparse reconstruction and the cached-73-base-map, geometric-consistency-off OpenMVS fusion/rough reconstruction. The earlier cache-resume `-002` rough attempt failed at its output cap and is **not** accepted as a mesh source. This continuation neither reruns the original pipeline end-to-end nor establishes metric scale or mesh quality.

The runner is [`openmvg_bunny_high_finish.py`](../scripts/classical_backend/openmvg_bunny_high_finish.py). Its default mode is read-only preflight. It consumes only the `-003` fusion source. That run produced native `dense.mvs`, `dense.ply`, and `mesh.ply`, but its top-level `result.json` was marked `failed` solely because the rough script expected an absent `mesh.mvs`. The failed receipt remains untouched. Preflight requires its exact SHA-256, a separate `openmvg_bunny_high_rough_rescue_v1` sidecar with status `native_rough_complete_pending_visual_review` binding that failed receipt and the three actual native artifacts, and a **third, independent positive visual review receipt**. It also checks both successful native stages, positive cached-fusion checkpoint, and frozen `--geometric-iters 0` / resolution-level 3. No visual approval is assumed here. Review must inspect native PLY validity **and** recognizable bunny shape. A native parse alone, a successful exit code, or the earlier camera approval does not satisfy this gate.

The review receipt is a JSON file with these required fields:

```json
{
  "schema": "openmvg_bunny_high_rough_visual_review_v1",
  "decision": "approved_for_refine_texture",
  "native_mesh_valid": true,
  "visual_object_shape_reviewed": true,
  "reviewer": "named reviewer",
  "rescue_receipt_sha256": "64 lowercase hex characters",
  "dense_mvs_sha256": "64 lowercase hex characters",
  "mesh_ply_sha256": "64 lowercase hex characters"
}
```

All three receipt **paths and caller-provided hashes** must match before any output path is created. The runner cross-checks the rescue sidecar against the failed rough receipt's complete native stages, log/artifact hashes and output inventory, all 73 staged photos, positive native dense/mesh counts, executable native binaries, and a fresh output root. It deliberately does **not** require `mesh.mvs`, which the native mesh stage did not emit. It copies only the hash-verified 73 photos plus `dense.mvs` and `mesh.ply`; no scanner, ground truth, or external camera poses enter this continuation.

Planned root: `/Volumes/backups/code/crispmesh-data/openmvg-bunny-high-openmvs-refine-004`. Native stages are OpenMVS `RefineMesh` (`dense.mvs` plus `mesh.ply`, resolution level 2, one scale) and then `TextureMesh` (`dense.mvs` plus `refined.ply`, OBJ export). Both request two threads and run under macOS `caffeinate`. The whole continuation, including copying, has a 20-minute **host wall-clock** deadline; native supervision also checks process elapsed time so a sleeping Mac cannot silently turn a 20-minute trial into an hours-long trial. Output is capped at 256 MiB with 16 MiB per-stage logs, sampled 4 GiB process-tree RSS, and an 11 GiB free-space floor on source, workspace and output disks. The exact 73 PNG, `dense.mvs`, and `mesh.ply` copy sum from `-003` is 126,104,220 bytes (120.262 MiB), leaving 135.738 MiB for refinement, texture and logs under the cap. Preflight requires at least 96 MiB after copying and another 256 MiB of available disk beyond the 11 GiB reserve. A failure leaves its separate output and receipt for diagnosis; it does not mutate or remove the rough source.

Once the native and visual review has actually been issued, the read-only check is:

```sh
python3 -m scripts.classical_backend.openmvg_bunny_high_finish \
  --source-receipt-sha256 ROUGH_RESULT_SHA256 \
  --rescue-receipt /absolute/path/to/rescue.json \
  --rescue-receipt-sha256 RESCUE_SHA256 \
  --review-receipt /absolute/path/to/review.json \
  --review-receipt-sha256 REVIEW_SHA256
```

Only after independent review of that preflight should `--execute` be added. A successful receipt would say `complete_conditional_continuation_only`, with `quality_accepted`, `metric_scale_verified`, and `shipping_approved` all false. The texture needs separate UV/atlas, geometry, and foreground/background inspection; completion is not a quality ranking against Apple, COLMAP, MVE, KIRI, or the scanner.

The synthetic preflight suite is `python3 -m unittest scripts.classical_backend.test_openmvg_bunny_high_finish -v`.
