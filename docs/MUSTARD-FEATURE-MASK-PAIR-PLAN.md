# Mustard feature-mask camera ablation: completed, negative result

This is a two-arm **training-only sparse SfM** comparison. The accepted SAM
coarse support is the control; the root-reviewed photo-refined v2 is approved
only as a bounded coarse feature-support ablation. The 12 held-out photos,
scanner mesh, sensor depth, prior camera models, native dense pipeline and
reference scores are outside both arms. Earlier mustard SfM and mask outputs
are immutable. The one approved live pair has now run; results and the
dense-stage decision are below.

The read-only [wrapper](../scripts/classical_backend/mustard_mask_pair.py)
is frozen at SHA-256
`47d5f5db014ee3fb249967f8ffbd5fd8d333c25b728d4af32488fe930cf8b584`.
Its
preflight checks the 48-name sealed train list, exact staged JPEG/SAM PNG bytes,
the v2 report and root visual decision, binary 1280×1024 masks, no added v2
pixels, preservation of all prior pixels at `y < 435`, and the exact per-view
removal counts. It rejects symlinks, missing/extra names, held-out names and
occupied output paths. It verifies the sealed `run.py`, interpreter, PyCOLMAP
extension and all OpenMVS binary hashes. The OpenMVS executables are checked
because `run.py` validates its toolchain; `--stop-after-sfm` prevents invoking
them. The existing producer copies each arm's selected inputs to its own fresh
external directory and rehashes those sources afterward. No source database
or prior experiment is opened for mutation.

Both commands use the exact same 48 original JPEG bytes and ordering, CPU
SIFT with maximum 1,800 features and 1,200-pixel image size, exhaustive
matching, seed `20260927`, shared `SIMPLE_RADIAL` camera, automatic initial
pair, fixed-initial intrinsics policy, up to five mapper models, minimum model
size 10, and 70% registration gate. The runner's ImageReader sets initial
focal factor 1.2; the previous same-profile run saved
`[f=1536,cx=640,cy=512,k=0]` on 1280×1024 images. Its
`fixed-initial` policy disables focal, principal-point and extra-parameter
refinement during bundle adjustment and focal/extra refinement during
absolute-pose fitting. This is a heuristic initial camera, **not measured
calibration**. Neither arm supplies poses, camera intrinsics, a manual seed
pair, or a sparse model. The only algorithmic input difference is
`--pose-mask-dir`: staged accepted SAM masks versus the 48 v2 masks. The
separate `--output` paths distinguish artifacts; all other flags, including
4.5-minute per-arm timeout, are identical. One wrapper invocation launches
arms serially within a 10-minute combined wall limit, even if the first arm
fails its registration gate; no threshold, seed or option retry follows.

The fresh outputs are
`/Volumes/backups/code/crispmesh-data/mustard-feature-mask-pair-001-sam` and
`...-v2`, with a separate
`/Volumes/backups/code/crispmesh-data/mustard-feature-mask-pair-001-receipt.json`.
Each arm is capped at 0.45 GiB (483,183,820 bytes), so the two producer
outputs plus a ≤1 MiB receipt stay below 1 GiB. Each stage has a 16 MiB log,
4 GiB sampled child RSS and two CPU threads. The wrapper requires at least
10 GiB free on the internal volume and **10 GiB plus the entire 1 GiB output
allowance** on external storage before launch, checks again before each arm,
and records both floors after the second arm. The producer itself monitors both
volumes and the per-arm limit during stages. Temporary files stay on the
external volume. The wrapper's fresh receipt saves the preflight, exact
commands, each arm's exit/producer status, effective PyCOLMAP options,
SHA-256 of every output file and all source photo/mask hashes before and after.
A failed/partial arm remains retained, not silently replaced or promoted. The
wrapper marks the pair `completed` only if **both** producer processes exit
zero **and** both producer reports say `sparse_complete`, plus unchanged sources,
bounded total output and the final disk floors. A zero exit with any other
producer status is `partial_or_failed`.

The read-only preflight passed with 48 verified training views, 23,711,059,968
bytes internal and 17,118,318,592 bytes external free. Four focused wrapper
tests passed, including a simulated failure after receipt creation that seals
`partial_or_failed` and a zero-exit/non-`sparse_complete` rejection. After approved
execution, compare registered names/count, verified-pair graph, tracks with
distinct-view counts, reciprocal track links, reprojection residual
denominators, saved intrinsics and acquisition-order orbit fold diagnostics.
No mesh-quality claim follows from registration count. Do not run native dense
from this plan; any dense mask ablation requires a separately selected, frozen,
plausible common camera model and its own plan.

The pre-run read-only preflight was invoked from the repository root as:

```sh
PYTHONDONTWRITEBYTECODE=1 .local-tools/colmap-sparse/venv/bin/python \
  -m scripts.classical_backend.mustard_mask_pair
```

The one `--run` invocation was authorized by root for wrapper SHA-256
`47d5f5db014ee3fb249967f8ffbd5fd8d333c25b728d4af32488fe930cf8b584`.
There was no retry or setting change. The preflight command above is read-only
but now correctly rejects the occupied output names; the same `--run` command
must not be used again.

## Sealed paired outcome and independent audit

The fresh [receipt](</Volumes/backups/code/crispmesh-data/mustard-feature-mask-pair-001-receipt.json>) has SHA-256
`3b6481f06f63506fdd2ab5c452a15f3e71c139d2ce8b722ef6e849ea0f12d9a5`
and status `completed`. The [SAM control result](</Volumes/backups/code/crispmesh-data/mustard-feature-mask-pair-001-sam/result.json>) SHA-256 is
`4d6ce1bba0534c25035168d8c82565cf99ad2415c5c7327fb7ca6f0e6c4401d3`;
the [v2 result](</Volumes/backups/code/crispmesh-data/mustard-feature-mask-pair-001-v2/result.json>) SHA-256 is
`cf56fab6fd4e51188b1ffabb7718f3e94fcdb0ea380e2892c04c5a6f9d7ca9ea`.
Both producers independently say `sparse_complete`, with one saved candidate
model each and 48/48 registered train views. Independent rehashing found
zero mismatches among 157 source files and 227 output files. The 48 copied
photo hashes match across arms; all 48 copied mask hashes differ. The saved
effective PyCOLMAP options differ only at `image_reader.mask_path`. Both saved
camera files have the same `[1536,640,512,0]` heuristic camera. Total new
output was 102,960,993 bytes. Final free space was 23,701,704,704 bytes
internal and 17,014,050,816 bytes external, above both 10 GiB floors.

Read-only diagnostics used each sealed COLMAP database and binary sparse model,
including model consistency checks and reciprocal 2D↔3D track links. A
nonempty verified two-view geometry defines a graph edge. Distinct-view track
support counts a 3D point only if observed in at least three different images;
repeated observations from one image are separately disclosed. Reprojection
values are L2 pixels over all finite saved track observations; they compare
different track sets and are selection-biased.

| Diagnostic | SAM control | Photo-refined v2 |
| --- | ---: | ---: |
| Nonempty verified pairs / connected component | 404 / 48 views | 403 / 48 views |
| Sparse points | 993 | 943 |
| Tracks with ≥3 distinct views / point denominator | 774 / 993 | 774 / 943 |
| Tracks repeating an image | 37 | 45 |
| Finite reprojections / all track observations | 4,916 / 4,916 | 4,961 / 4,961 |
| Reprojection mean / median / p95, px | 1.133 / 0.947 / 2.789 | 1.091 / 0.882 / 2.752 |
| Adjacent center step / median radius, median / p95 / max (47 edges) | 0.133 / 1.775 / 2.268 | 0.124 / 1.409 / 2.359 |
| Adjacent full orientation step, median / p95 / max (47 edges) | 6.15° / 57.65° / 93.37° | 6.01° / 54.36° / 99.51° |
| Opposing-label center distance / median radius, median / p95 (24 pairs) | 0.717 / 2.398 | 0.061 / 1.524 |
| Opposing-label full orientation angle, median / p95 (24 pairs) | 23.84° / 102.18° | 3.12° / 64.53° |

The NP3 suffix/turntable angle labels were used **only after reconstruction**
to order estimated cameras and identify nominal adjacent/opposing photo pairs;
they did not enter masking, matching, initialization or mapping. They are not
physical camera-pose ground truth. Even so, both image-estimated trajectories
show camera alias warnings. In the SAM arm, `NP3_078/258` centers are only
0.052 median radii apart with 2.08° full-orientation difference; in v2 they
are 0.010 radii and 2.10°. V2 additionally places `NP3_096/276` at 0.014
radii and 1.91°. The adjacent `NP3_282→288` step jumps 2.268 radii/93.37°
in SAM and 2.359 radii/99.51° in v2. These are incompatible with treating
48/48 registration as proof of a plausible full orbit. V2 has a much smaller
median opposing-label separation than the control, so the boundary trim did
not cure the fold and may have made this one trajectory more aliased. Lower
conditional reprojection error and point counts do not establish a better
shape or a causal generalization across runs.

Decision: retain both sparse arms as a **negative feature-mask ablation**.
Neither camera model is accepted for native dense reconstruction from this
experiment. No dense stage, held-out localization, sensor comparison, scanner
fit or reference score was run. A future camera correction needs a separately
frozen image-only hypothesis and camera-plausibility gate before revisiting
dense masks; this trial does not authorize a seed or threshold sweep.
