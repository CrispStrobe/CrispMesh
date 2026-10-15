# Proposed mustard sparse-SfM mask comparison

Status: **paired 48-view sparse comparison executed; both arms failed**. The
original SAM inventory was rejected; a separately reviewed 43+3+2 candidate
was composed and staged before the frozen serial runs. This experiment
compares image-estimated sparse reconstruction, not mesh quality.

## Immutable input boundary

The [VPS-validated mustard evaluation package](../build-opencv/ycb-evaluation-vps-001/mustard-package.json)
(SHA-256 `45b6ed1430627a747eaeb8d4dff7d2c49502083cca0a7914e25aacd45b594425`)
binds 48 training originals and 12 held-out originals. Its acquisition-manifest
SHA-256 is `c48998dffc7798e7d21f275383b1c67dc2e5e9313a8bfe699f72303f61583b52`.
The 48 JPEGs total **48,683,099 bytes**. Their exact ordered basenames are:

```text
NP3_000.jpg NP3_006.jpg NP3_012.jpg NP3_018.jpg NP3_030.jpg NP3_036.jpg
NP3_042.jpg NP3_048.jpg NP3_060.jpg NP3_066.jpg NP3_072.jpg NP3_078.jpg
NP3_090.jpg NP3_096.jpg NP3_102.jpg NP3_108.jpg NP3_120.jpg NP3_126.jpg
NP3_132.jpg NP3_138.jpg NP3_150.jpg NP3_156.jpg NP3_162.jpg NP3_168.jpg
NP3_180.jpg NP3_186.jpg NP3_192.jpg NP3_198.jpg NP3_210.jpg NP3_216.jpg
NP3_222.jpg NP3_228.jpg NP3_240.jpg NP3_246.jpg NP3_252.jpg NP3_258.jpg
NP3_270.jpg NP3_276.jpg NP3_282.jpg NP3_288.jpg NP3_300.jpg NP3_306.jpg
NP3_312.jpg NP3_318.jpg NP3_330.jpg NP3_336.jpg NP3_342.jpg NP3_348.jpg
```

The [frozen split rule](YCB-EVALUATION-PROTOCOL.md) excludes angles
24°, 54°, 84°, 114°, 144°, 174°, 204°, 234°, 264°, 294°, 324°, and 354°.
Neither those JPEGs nor Berkeley depth/poses, supplied masks, or Google scan
geometry may enter either arm. Angle labels select the split only; they do not
seed camera estimation. Stage exactly the 48 JPEG bytes and, after acceptance,
the 48 cleaned SAM PNGs in a **fresh ≤150 MiB** external-volume directory.
After full-view QA acceptance, read the complete per-frame inventory's
`frames/IMAGE_NAME/clean.png` files and stage them in one `masks/` directory
with exact names `IMAGE_NAME.png` (for example `NP3_000.jpg.png`), as required
by the existing PyCOLMAP reader. Rehash each staged photo against the package
and each cleaned mask against the sealed complete inventory and accepted QA
hashes; reject missing, duplicate, linked, extra, or changed files. Record
the accepted QA decision and complete-inventory SHA.
Make a basename-only `train-names.txt` in the order above and bind its hash.
The [train-only staging helper](../scripts/classical_backend/mustard_stage.py)
now implements these checks, including a fresh exact output, decoded binary
mask/photo dimensions, hashes and a ≤150 MiB copy cap; its tests are in
[`test_mustard_stage.py`](../scripts/classical_backend/test_mustard_stage.py).
It deliberately requires a separately sealed 48-mask inventory and explicit
root visual-QA acceptance. The reviewed v2 candidate below satisfied that
gate before staging. A separate photo-only
48-TRAIN stage completed at
`/Volumes/backups/code/crispmesh-data/sam21-m1-train48-001` (report SHA-256
`d6e72143fc02ee3d18dd8961ae508417bf122cbdbc24ebc4465e207a30997861`);
it contains no masks or held-out images and does not satisfy the mask/QA gate.

## Five-view correction and new full-set decision

The sealed original inventory (SHA-256
`b47e8caec7d05fae7bb84ae9b3eec7d4a35b1457ca9df67a3b984cbaaeed8a98`)
remains rejected by the [tracked root QA](../tests/datasets/sam21_mustard_m1_full_review.json)
(SHA-256 `4084e8e458b55c566dd138f9d4b8b853ab2c68bfc20f5f831447a0cc99e9528a`).
Only `NP3_318.jpg`, `NP3_330.jpg`, `NP3_336.jpg`, `NP3_342.jpg`, and
`NP3_348.jpg` may be replaced. The [separate correction prompts](../tests/datasets/sam21_mustard_m1_board5_prompts.json)
(SHA-256 `8cc0b6fa49d977194870ef91f10b83a793294d6b3903cb1e08643016bdc9b4f8`)
retain the original three points and add a negative board point `(620,330,0)`
for those five. They do **not** change the original 43 prompts or make the
original global prompt SHA a valid label for corrected masks.

The [candidate composer](../scripts/classical_backend/mustard_candidate48.py)
requires a completed five-view worker manifest **and** successful resource
supervisor, fixed SAM checkpoint/source/runner seals, an independent root
review accepting exactly those five corrected masks, and exact original
photo and per-frame hashes. It checks all 48 original raw/clean masks, all
five replacement raw/clean masks, and metadata before and after copying.
Its fresh `sam21_mustard_m1_composed_candidate_v1` inventory records an
ordered 43 `base43` / 5 `board5` origin and per-image prompt SHA; it remains
`generated_unreviewed`. It cannot inherit the rejection's acceptance.
The [stager](../scripts/classical_backend/mustard_stage.py) recognizes this
new lane only with a *second*, independent
`mustard_sam_composed_candidate_qa_v1` decision accepting and hashing all
48 composed cleaned masks. The old inventory lane and its QA contract remain
separate. Tests use tiny synthetic masks and do not substitute for the root's
image review.

The first corrected five-view attempt at
`/Volumes/backups/code/crispmesh-data/sam21-mustard-m1-board5-001` **failed**.
Its manifest SHA-256 is
`1008fc7bdbaa75ac33f17f8ddf44eaa275b3e98eb21466906060ebc3300c638c`
(`partial_unusable`); its separate supervisor SHA-256 is
`b4485b72c539d59b6b3229917f0099ceb11359cdb8c9228bd0553208989619ce`
(`failed`, worker exit 1, 27.78 seconds, 1,436,656 KiB sampled peak RSS).
Three masks completed unreviewed (318°, 330°, 348°). The 336° mask failed
the four-point foreground/background check; the 342° mask failed the
predeclared cleanup limit of 10% raw foreground removal. These failures are
not a visual-QA acceptance or an invitation to use a partial set. The
v1 whole-five composer requires both a five-of-five manifest and a successful
supervisor, so that *v1 route* could not compose. The later v2 selector
retained the failed parent status and used only separately accepted rows.

Root subsequently accepted only the three complete board5-001 rows (318°,
330°, 348°) for possible coarse feature support in a
[separate subset review](../tests/datasets/sam21_mustard_board5_subset_review.json)
(SHA-256 `940f7e76be973edb74b09a73be49e254911a99a8b8a7a3b654eeada270babecd`)
and [bounded-parent audit](../tests/datasets/sam21_mustard_board5_resource_audit.json)
(SHA-256 `bf058458fd0b5c3bc5c51b2840936c1ce9c40f41d2249867cec0b80ccef6088e`).
The five-view parent remains **partial/failed**. A distinct declarative
[two-view recipe](../tests/datasets/sam21_mustard_m1_two_view_recipe.json)
(SHA-256 `38f2a40af084df4c177925bfcfad7b5ebe5f131388283d14d15f5e690a8d0367`)
produced masks for 336° and 342° in a fresh run at
`/Volumes/backups/code/crispmesh-data/sam21-mustard-m1-recipe-001` (manifest
SHA-256 `361d32f4342fa193d5bfed68289f0a44151b3768b54d2a30998dd4a6b11c6f01`,
supervisor SHA-256 `c9124c9d1fe921396fda801f15a7f2cb9a7022a6d1829494aad9bc12ff7e703d`).
Both new producer statuses were `generated_unreviewed`, then the two masks
received a separate root subset QA. Its smaller box
`[520,365,665,600]` intentionally allows the negative point `(620,330,0)`
outside the box but inside the original image; pinned SAM2 transforms box
corners and point coordinates independently. This is valid input syntax,
not evidence of mask quality.

The same composer/stager now contain a separate `--selection` v2 lane for an
exact ordered 43 original + 3 accepted partial-parent + 2 accepted ROI-recipe
candidate. It preserves each parent status, per-image recipe-row SHA, source
manifest/supervisor SHA, reviewed contact-sheet SHA and subset QA. The
candidate was composed fresh at
`/Volumes/backups/code/crispmesh-data/sam21-mustard-m1-candidate48-002`
(inventory SHA-256 `c341e3798a792c83abbacac8a7f882a761c878cdd9dba693c02c5877bbb75751`),
with full-frame training RGB/mask sheet SHA-256
`fba2b0cc9c271857d7eb4170e34889c1c96bf79c2cfc1345df040c0bfe2496be`.
Root's [independent full-48 QA-v2](../tests/datasets/sam21_mustard_candidate48_v2_review.json)
(SHA-256 `c07a62441a4846f53619d28d16f931bfd3a0e85f99a26df83bc6b664cdd8a2da`)
accepted the exact 48-mask inventory and sheet only for coarse pose support.
The fresh train-only stage at
`/Volumes/backups/code/crispmesh-data/mustard-sfm-train-001` copied exactly
48,794,098 bytes; report SHA-256
`bbcba624f6a51f82e0ebdebf6e7e4cda10804bc8df34cfd0d5b3242983a225d0`.
No held-out photo, depth, supplied pose, or reference mesh entered staging.

For a *future* independently accepted complete correction, run from the
repository root with the pinned interpreter
`.local-tools/colmap-sparse/venv/bin/python`
and `PYTHONDONTWRITEBYTECODE=1`. The read-only composer module is
`-m scripts.classical_backend.mustard_candidate48`
with `--correction-root <fresh-complete-board5-root>`,
`--correction-manifest-sha256 <sealed-new-manifest-sha>`,
`--correction-prompts tests/datasets/sam21_mustard_m1_board5_prompts.json`,
`--correction-prompts-sha256 8cc0b6fa49d977194870ef91f10b83a793294d6b3903cb1e08643016bdc9b4f8`,
`--correction-qa <tracked-five-view-review>` and
`--correction-qa-sha256 <accepted-review-sha>`. Only separate root approval
permits repeating with `--compose`; that writes fresh
`/Volumes/backups/code/crispmesh-data/sam21-mustard-m1-candidate48-001`.
The analogous read-only staging module is
`-m scripts.classical_backend.mustard_stage` with
`--train-photos /Volumes/backups/code/crispmesh-data/sam21-m1-train48-001/photos`,
`--inventory /Volumes/backups/code/crispmesh-data/sam21-mustard-m1-candidate48-001/complete_inventory.json`,
`--inventory-root /Volumes/backups/code/crispmesh-data/sam21-mustard-m1-candidate48-001`,
`--inventory-sha256 <composed-inventory-sha>`,
`--qa <tracked-full48-review>` and
`--qa-sha256 <accepted-full48-review-sha>`. Only separate approval permits
repeating with `--prepare`, producing fresh
`/Volumes/backups/code/crispmesh-data/mustard-sfm-train-001`. The currently
failed board5-001 artifact cannot satisfy these placeholders; a future
complete artifact requires its own fresh path and reviewed hashes.

There is no existing mustard SfM baseline. Prior cracker-box raw/masked trials
are settings precedent, not a cross-object baseline. The control must be a
fresh **unmasked run on these same 48 mustard photos**.

## Paired frozen run

Use the existing [`scripts.classical_backend.run`](../scripts/classical_backend/run.py)
with `--stop-after-sfm`, once without masks and once with the accepted mask
directory. Its source explicitly sets a single unknown shared camera,
`SIMPLE_RADIAL`, initial focal factor 1.2, CPU feature/matching, and the
chosen options below. Use the same pinned PyCOLMAP 3.11.1 executable and
OpenMVS binary-directory hashes in both runs; the CLI currently checks all
five OpenMVS tools even when stopping after SfM, although none is executed.
No supplied poses, calibrated intrinsics, manual initialization pair, or
reference-fitted transform are allowed. Run serially in fresh output paths.

Both commands share these explicit flags (not merely defaults):

```text
--images <staged-train-photos> --image-list <sealed-train-names.txt>
--max-views 48 --stop-after-sfm --camera-model SIMPLE_RADIAL
--matching sequential --sequential-overlap 8
--sift-max-features 1800 --sift-max-image-size 1200 --seed 20260927
--sfm-max-models 5 --sfm-min-model-size 10 --min-registered-fraction 0.7
--max-threads 2 --max-gib 2 --max-rss-gib 4 --max-log-mib 32
--timeout-minutes 10 --python <pinned-pycolmap-venv-python>
--binary-dir <verified-openmvs-bin-directory>
```

Only the masked command adds `--pose-mask-dir <staged-cleaned-masks>`; each
uses its own `--output <fresh-external-path>`. These matching, SIFT, and seed
values reproduce the earlier image-only object-motion **option profile**;
the five-model retry is the existing classical runner's generic policy and
is held equal in both arms. Do not select an initial pair based on results.
The runner captures effective `pycolmap-options.json`, input hashes, stages,
and `sfm.json`, including registered and missing names and candidate models.
An arm below 70% registration is a failed *preset gate*, but retain its
diagnostic counts; do not erase or relabel it as a completed result.
The prepared paired output names are fresh
`/Volumes/backups/code/crispmesh-data/mustard-sfm-raw-001` and
`/Volumes/backups/code/crispmesh-data/mustard-sfm-masked-001`.
Both use `/Volumes/backups/code/crispmesh-data/mustard-sfm-train-001/images`
for `--images`, that directory's `train-names.txt` for `--image-list`,
the absolute interpreter above for `--python`, and
`.local-tools/classical-backend/bin`
for `--binary-dir`. The masked-only `--pose-mask-dir` is
`/Volumes/backups/code/crispmesh-data/mustard-sfm-train-001/masks`.
Set `TMPDIR=/Volumes/backups/code/crispmesh-data` for each run. Both frozen
arms were subsequently launched with the same listed settings and image order;
their sealed outcomes are reported below. These commands describe that paired
run, not a new launch request.

Each run is capped at **2 GiB output**, **10 minutes**, **32 MiB per stage
log**, **4 GiB sampled child RSS**, and two CPU threads. The runner already
enforces a 10 GiB free-space floor on its output filesystem. Before launch,
verify ≥10 GiB plus the ≤150 MiB stage and both 2 GiB run allowances on the
external volume, and ≥10 GiB on the Mac internal volume. The current
classical runner now checks the distinct internal device at preflight,
copy-time, during each child stage and post-stage, in addition to its output
device. The [device-selection test](../scripts/classical_backend/test_mustard_stage.py)
and [stage-stop test](../scripts/brush_backend/test_masked_smoke.py) must
remain passing before launch. Set process temporary files to
the external run directory. The VPS system Python currently lacks PyCOLMAP;
the existing Mac venv is the shorter runnable path after a train-only staged
transfer. No transfer or installation is authorized by this document.

## Read-only comparison after sealing

Report, for **both** arms: registered names/count out of 48; candidate-model
counts; verified two-view-match graph connected components including isolated
images; sparse point count; per-image triangulated observations; track-length
median, p95, and fraction with ≥3 observations; and mean/median/p95 of
per-observation L2 reprojection error in pixels. The graph uses one vertex
per training image and an edge for a nonempty verified `two_view_geometries` row, with that
definition fixed before reading outputs. Project every retained 3D track
point into each registered observation's camera and compare to its measured
2D coordinate; report the number of finite observation residuals. Read
tracks and reprojection from the saved binary COLMAP models, never from
held-out or scan data. The runner does **not** itself emit all component,
track, or reprojection statistics. The separate
[`sparse_sfm_metrics.py`](../scripts/object_dataset/sparse_sfm_metrics.py)
now reads only a sealed run's training package, producer report, SQLite
feature/match database and binary sparse model, hashes them before/after,
and reports these metrics with finite/invalid reprojection denominators.
Its analytic and tiny native PyCOLMAP tests are in
[`test_sparse_sfm_metrics.py`](../scripts/object_dataset/test_sparse_sfm_metrics.py).
The two arms did not produce an accepted sparse export, so the failure-aware
path of that evaluator was used on their sealed results. It reads a
sidecar-free, byte-identical database snapshot, verifies the original database
and archived producer source before and after, and never treats a rejected
candidate as a successful model.
Report all denominators and any missing models. Lower reprojection error with
fewer tracks or cameras is not automatically better. Camera gauge is arbitrary
Sim(3), and a sparse success is neither an object mesh nor physical accuracy.

### Sealed paired outcome (training images only)

| Arm | Producer result | Verified-pair graph | Saved geometry |
| --- | --- | --- | --- |
| Raw | Failed: no initial pair/model | 348 nonempty verified-pair edges; all 48 images in one connected component | None; registration, tracks, and reprojection are unavailable, not zero |
| Masked | Failed the preset three-camera gate | 229 nonempty verified-pair edges; all 48 images in one connected component | Rejected candidate only: 2/48 registered (`NP3_006`, `NP3_030`), 69 points, 138/138 finite track-observation projections |
| Masked, fixed-initial intrinsics ablation | Sparse stage complete: 37/48 registered, not quality-accepted | 228 nonempty verified-pair edges; all 48 images in one connected component | 717 points; 3,437/3,437 finite track-observation projections, with repeated-image track anomalies disclosed below |
| Masked, fixed-initial + exhaustive matching ablation | Sparse stage complete: 48/48 registered, not quality-accepted | 400 nonempty verified-pair edges; all 48 images in one connected component | 939 points; 4,815/4,815 finite track-observation projections, with repeated-image track anomalies disclosed below |

For the **rejected two-camera candidate**, reprojection L2 is mean 0.4329 px,
median 0.2588 px, p95 1.4115 px over 138 observations; each of the 69 tracks
has length two. These small residuals on two selected views are not evidence of
48-view reconstruction quality. Its saved SIMPLE_RADIAL intrinsics are
`f=336.1569`, `cx=640`, `cy=512`, `k=5.6530` for 1280×1024 images. The radial
parameter exceeds the mapper's recorded `max_extra_param=1.0`; this is a
diagnostic of an implausible rejected candidate, not a proven sole cause of
the failure. The raw arm's connected match graph likewise does not imply a
recoverable initial pair. No held-out image, reference scan, or GT was used to
select either arm or compute these metrics.

The fixed-initial masked ablation held the image reader's initial intrinsics
fixed during mapping; its single saved SIMPLE_RADIAL camera is
`[f=1536,cx=640,cy=512,k=0]` on 1280×1024 images. This is the fixed initial
value (consistent with the 1.2×width heuristic fallback), **not measured
calibration**. Its registered views omit eleven
train images, including the contiguous tail `NP3_276` through `NP3_348`.
Of 717 tracks, 604 (84.24%) contain at least three **distinct views**;
distinct-view median length is 4 and p95 is 10. Reprojection L2 across all
3,437 saved track observations is mean 1.1590 px, median 0.9498 px, p95
2.8418 px. Crucially, 31 tracks contain 53 extra observations from images
already present in those tracks (different 2D keypoints). The evaluator counts
all observations, verifies every 2D↔3D backlink, and separately reports
distinct-view track lengths. This saved-model anomaly prevents treating the
small residuals or the evaluator's `status=complete` as an integrity-clean or
object-quality success. The policy ablation changed mapping intrinsics, not
the frozen images/masks/matching settings; it is a distinct run, not a paired
raw-versus-masked effect estimate. No object mesh or held-out camera evaluation
was produced here.

One final **matching-only** ablation retained the same 48 masked training
photos, SIMPLE_RADIAL single camera, fixed-initial policy, feature cap,
resolution, seed, model retry bounds, and registration gate, but switched
sequential matching to exhaustive matching. It registered 48/48 training
views with 939 points; its camera remained `[1536,640,512,0]`. Of 939
tracks, 753 (80.19%) contain at least three distinct views; distinct-view
median is 4 and p95 is 12. Across 4,815 saved track observations,
reprojection L2 is mean 1.0839 px, median 0.8677 px, p95 2.7589 px. Here
42 tracks contain 81 extra observations from images already represented in
their tracks. More registration and points with exhaustive matching are a
training-SfM diagnostic, **not** a held-out pose or shape-quality result;
the repeated-image anomalies remain, and initial intrinsics are still not
independently calibrated. Worker stage wall times overlapped an unrelated
test suite, so they must not be ranked as controlled speed measurements.

The full small reports are preserved as
[`raw failure metrics v2`](../tests/evidence/mustard-sfm-raw-failure-metrics-v2.json)
(SHA-256 `4ca65c22c56cc78a72476a0523dedc2d3b49389f8054f4c18226b942c5fe5983`),
[`masked failure metrics v2`](../tests/evidence/mustard-sfm-masked-failure-metrics-v2.json)
(SHA-256 `8b81729269be28df76bf6b9b73b85821b6e12b747e428aeb5bdeaab724a5e205`),
[`fixed-initial sparse metrics`](../tests/evidence/mustard-sfm-masked-fixed-initial-sparse-metrics.json)
(SHA-256 `a2242996cb9d0c48ff1281665461769b8e4041f5550c5c5f42a110d25b2085af`),
and [`fixed-initial exhaustive sparse metrics`](../tests/evidence/mustard-sfm-masked-fixed-exhaustive-sparse-metrics.json)
(SHA-256 `76bc60d9aba999b50a73ad51ffbe7d08e0c0a8a2bd68f1d45279ac51f7a1cf26`).
These are final-helper reports; the earlier external v1 failure diagnostics
remain as immutable superseded evidence, not the cited comparison.
They bind the original producer reports, archived historical runner source,
database snapshots for the failed runs, logs, effective options, and saved
binary models. The original producer databases and sidecars were not modified
by this analysis. The fixed-initial run does not retroactively change the
paired failure result.

Do not inspect held-out photos to choose masks or SfM options. A later
held-out image evaluation needs separately estimated test cameras and its
own leakage disclosure; this paired training-only result is not that test.

### Interpretation correction after the stock Sceaux control

COLMAP permits nearby observations from the same image within one 3D track;
its author [explicitly documents this behavior](https://groups.google.com/g/colmap/c/kcm32t15qEQ).
The subsequent stock Sceaux run also contains such tracks while closely agreeing
with independent software cameras. Therefore the earlier labels "anomalies" and
"structural repair" below are too broad: a repeated image alone does **not** prove
an invalid COLMAP model or incorrect geometry. Count distinct views for support,
check reprojection and reciprocal image/track references, and handle downstream
format constraints separately. The historical candidate and all its measured
changes remain preserved; its geometry edits were not shown to improve accuracy.
The mustard camera-fold warning remains independent of this correction.

### Bounded duplicate-view track repair candidate (historical experiment)

A separate, immutable candidate repaired the 42 tracks that repeated an image
in the fixed-initial exhaustive model. Within each affected track, it retained
the observation with the lowest finite saved-geometry reprojection residual
per image (tie: lower 2D index), removed the other 81 observations, and
retriangulated only those 42 tracks with frozen camera poses/intrinsics. The
other 897 point XYZ coordinates and observation links remained exactly
unchanged; cached point errors were recomputed. This is a structural repair,
not an independent optimization of object shape.

The candidate retained all 939 points, all 48 cameras, and 4,734/4,815
observations (98.32%). It has zero repeated-image tracks and passed every
predeclared integrity/dense-eligibility gate. On the **same original 4,815
observations, including discarded links**, mean reprojection was 1.083929 px
before and 1.084016 px after; p95 was 2.758900 px before and 2.805505 px
after. Selected-only residuals use 4,734 observations and must not be read as
an independent accuracy gain. No reference mesh, held-out photo, or supplied
camera calibration informed the selection rule. The fixed initial focal
length remains unvalidated physical calibration, and neither these gates nor
root's acceptance for a bounded dense trial certify mesh quality.

The byte-exact [repair report](../tests/evidence/mustard-sparse-repair-report.json)
(SHA-256 `32beea7955df0642cc87092afc39e96647adf9fbab582eaf14d6a6d662ef5ef8`),
[supervisor report](../tests/evidence/mustard-sparse-repair-supervisor.json)
(SHA-256 `b75bb6a23c7ff0b4b3d7dc1216792aa173308c32fabbbbb7700af8922aa97d7b`),
and [root QA](../tests/evidence/mustard-sparse-repair-root-qa.json) bind the
source and output model hashes. The original sparse model is preserved; the
candidate is under the external `mustard-sparse-repair-001/model` directory.

## Next scoped quality tasks

1. Run the root-approved bounded dense trial from the separately repaired
   sparse candidate, then evaluate geometry and source-mask support without
   inferring accuracy from camera registration or track integrity alone.
2. Evaluate guarded delayed intrinsics refinement only after that structural
   audit. Fixed initial intrinsics stabilized this capture but are not known
   calibration; reject collapsed focal/radial solutions and preserve the fixed
   baseline. Do not tune against the reference mesh or held-out photographs.
3. Run a bounded dense/mesh pipeline from accepted camera candidates and score
   independent reference-surface accuracy and completeness together. Full
   camera registration is not the acceptance criterion for that milestone.
