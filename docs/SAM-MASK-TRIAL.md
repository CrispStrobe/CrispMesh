# SAM 2.1 tiny mustard mask smoke

Status: **three training-view smoke completed, visually unapproved**. This is a
photo-only, prompted segmentation candidate for coarse pose support, not a
validated silhouette or a ready reconstruction mask set. No held-out photo,
reference mesh, supplied mask, depth image, calibration, or pose was read by the
runner. No SfM/dense run consumed these masks. The fixed box prompt was the
previously reviewed original-pixel ROI `[480,300,760,650]` in 1280×1024 RGB.
No prompt or threshold was selected by reconstruction or reference score.

## Pin and execution boundary

The [Meta SAM 2 repository](https://github.com/facebookresearch/sam2) was
cloned on the VPS only at Git commit
`2b90b9f5ceec907a1c18123530e92e794ad901a4` (clean tracked checkout).
Its actual Python/config/license file-inventory digest, as defined by
`source_digest()` in the runner, was
`c7eb4585a22dadd4f54ffd9134e3103c1951745a3b4631ceb0684b55742069e3`.
The official Meta Hugging Face checkpoint
[`facebook/sam2.1-hiera-tiny`](https://huggingface.co/facebook/sam2.1-hiera-tiny)
was fetched at repository revision `de431c4043854a71d8101e17995dfe596bf101a5`:
`sam2.1_hiera_tiny.pt`, 156,008,466 bytes, SHA-256
`7402e0d864fa82708a20fbd15bc84245c2f26dff0eb43a4b5b93452deb34be69`.
The observed digest equals the publisher's LFS SHA-256 metadata. Meta's
[license statement](https://github.com/facebookresearch/sam2/blob/main/README.md#license)
explicitly covers SAM 2 checkpoints and code under Apache-2.0; the HF model
card also labels this checkpoint Apache-2.0. That permissive license has no
noncommercial restriction, subject to its notice and other terms. This is a
license inventory, not legal certification.

The CPU VPS had Python 3.12.3, PyTorch 2.7.0+cpu, torchvision 0.22.0+cpu,
Pillow, and NumPy. Missing SAM2 runtime dependencies `hydra-core==1.3.2` and
`iopath==0.1.10` plus their dependencies were installed with `pip --target`
under `/mnt/storage/crispmesh-data/sam21-tiny-prereq-001/deps`; the attempted
venv on that storage mount failed because it could not create a `lib64`
symlink. No global package install, Mac model download, fast-volume scratch,
model conversion, or GPU was used. The source checkout occupied 65 MiB and
the complete prerequisite directory approximately 279 MiB. Before inference,
available RAM was ~3.45 GiB, above the runner's 2.5 GiB gate. The runner
sets PyTorch/OMP/MKL/OpenBLAS to two threads, caps the child at 1.5 GiB RSS,
300 s wall time, 1 MiB log and 20 MiB output, and requires at least 10 GiB
free on the output filesystem. Its monitor measures the worker process, not
arbitrary descendants; this model invocation created no observed extra worker.

`scripts/object_motion/sam_mask_trial.py` validates the pinned 48-train package,
but opens only the fixed smoke subset `NP3_000`, `NP3_006`, `NP3_012`. It
rehashes each source photo before and after decode and hashes the masks,
checkpoint, source inventory, package, runner, and shared verifier. The
checkpoint/source expected hashes were supplied as reviewed exact CLI values;
the code is not a general trusted-model allowlist. Outputs are full-resolution
binary `IMAGE_NAME.png` masks plus an original-ROI overlay/masked contact sheet.
The machine status is `complete_unreviewed`, never QA acceptance.

## Actual smoke and visual limitation

The first fresh run, `/mnt/storage/crispmesh-data/sam21-mustard-smoke-001`,
failed before writing a mask because the official predictor returns binary
float32 values while the initial validator accepted only bool/uint8. Its
failure manifest and supervisor report remain intact; elapsed 22.2 s and peak
RSS 925,028 KiB. A narrow interface correction accepted only finite exact
float32 0/1 and copied the RGB NumPy array before Torch conversion. The
model, checkpoint, prompt, and three photos did not change. Three unit tests
pass with an injected fake predictor, including exact binary output and fresh
output rejection.

The fresh run `/mnt/storage/crispmesh-data/sam21-mustard-smoke-002` completed
all three masks in 28.96 s, peak worker RSS 1,014,004 KiB (~0.97 GiB). Mask
support areas were 20,805, 20,694, and 20,579 pixels; inference took about
5.0–5.6 s per image. The [local copied review sheet](../.local-tools/sam21-mustard-review-002/smoke_overlay.jpg)
has SHA-256 `4aa05de85b75a8b2d2153af94236a76dd1be334e863aa8f60687396e0320e047`;
its matching manifest/supervisor were also copied there. The three full-size
masks remain on the VPS under `sam21-mustard-smoke-002/masks/`.

On inspection of the original-ROI sheet, the blue/red bottle labels are
retained far better than the rejected warm-color masks. However, a few
non-object pixels remain near the cap and checkerboard, especially in
`NP3_012`. This **does not pass full-view visual QA** and does not authorize
the 48-view run. Any future 48-mask candidate needs a separately approved,
frozen extension and all-48 visual review before pose matching; an exact
dense silhouette would require a separate contract.

## Optional connected-object cleanup smoke

The follow-on [cleanup helper](../scripts/object_motion/sam_mask_cleanup.py)
does **not** rerun SAM or alter the prompt. It takes the sealed raw-smoke
manifest (SHA-256
`6804b82de786e2488bf6dd2e5f776b59025fef019a4bb8d3101fefbef5cddb94`)
and its three exact-hash raw PNGs, then keeps only the sole largest
**8-connected** foreground component. It does not fill holes, erode, dilate,
smooth, or move surviving pixels. Equal-sized largest components, empty or
nonbinary masks, and removal over 10% are rejected as ambiguous. This is an
explicit single-connected-object assumption, **not** a general segmentation
guarantee: a truly detached visible object part would be removed. Both raw
byte-identical copies and cleaned masks are retained in separate directories.

One fresh VPS cleanup run completed at
`/mnt/storage/crispmesh-data/sam21-mustard-cleanup-smoke-001/`; manifest SHA-256
`789fe5a942d21b32a0967d2e8479a67873240ec6f5382a69651a5c7347aa3c07`.
It removed 5 pixels from `NP3_000`, 3 from `NP3_006`, and 31 from `NP3_012`
(the latter is one detached component); the retained fractions all exceed
99.84%. The [three-view raw-versus-cleaned crop](../.local-tools/sam21-mustard-cleanup-review-001/raw_vs_cleaned_crop.jpg)
has SHA-256 `ba3fa08e56a389055f6fa6eed9baccac0c40c4f55aaa4e8182e977e0c428e07a`.
It shows the detached `NP3_012` checkerboard speck removed while the bottle
and label remain visible. Six focused SAM/cleanup tests passed. This is only a
three-view QA candidate; the output remains `complete_unreviewed`, and no
48-view inference was part of that smoke. The later separately bounded 48-view
candidate below supersedes any suggestion that these three views establish
general mask quality.

## Frozen 48-training-view extension: rejected

After reviewing the cleaned three-view smoke, root approved one extension
using the **same box, SAM2 source/checkpoint, and largest-8-component rule**,
strictly on the 48 training photos. The [full runner](../scripts/object_motion/sam_mask_full.py)
fixes three sequential 16-view batches with no per-view prompt change,
two CPU threads, a 120 s/1.5 GiB worker RSS bound per batch, 420 s overall,
60 MiB output ceiling, and a new 2.5 GiB available-RAM check before each
batch. A failed or ambiguous cleaned mask is recorded `failed_unusable` while
its raw mask remains; no forced component is published. Source/model/helper
hashes are rebound before and after each batch. Review sheets are hash-checked
against batch manifests and show failed/missing tiles explicitly. Eight
focused SAM tests passed before this VPS run.

The fresh output is `/mnt/storage/crispmesh-data/sam21-mustard-full-001/`.
Batch 0 attempted 16 views in 113.97 s, peak RSS 1,120,960 KiB;
batch 1 attempted 16 in 116.50 s, peak RSS 1,122,640 KiB. Both respected
their bounds. The 48-view supervisor stopped **before launching batch 2**
because available RAM fell below 2.5 GiB at that pre-batch check. It retained
the 32 attempted views and wrote a `failed_or_partial_unusable` report, SHA-256
`0441b19a257dd424c3c77728bdee405efddf62ace9563d105d6d76ffcc67c936`.
The batch manifests have SHA-256
`aa37ec1e6dbed0c7141ab1637f086ac19a5ffec09634faa5a4d8dd2d2f9656a1`
and `fdb1f738609a247bb2bfdfe3a41e946e8aa3d26c8f24528bd2573a8b2d99ec6b`.
In those 32 views, 19 numerically cleaned and 13 were flagged unusable
because removing disconnected foreground would exceed the frozen 10% guard.
The unused batch 2 was **not** later resumed: root inspected the first 32 and
rejected the method outright, so more inference would not change this
candidate's decision.

The [raw](../.local-tools/sam21-mustard-full-review-001/training_raw_sheet.jpg),
[cleaned](../.local-tools/sam21-mustard-full-review-001/training_cleaned_sheet.jpg),
and [full-context](../.local-tools/sam21-mustard-full-review-001/training_full_context_sheet.jpg)
review sheets were copied locally with exact SHA-256 values
`5ae4f78ff36ed60586352081877e7053f84af23adc90caabc4c2316c937d8033`,
`61dc600c720e55a5a83f601222fd129f3a53c68c05fc21d5a40998c7542eee43`,
and `07db3aa1ecf26db3f460ec80fd7520ecbdd63c80fa1692ae7063b94d4c2fb33c`.
Root and the implementation agent independently found the decisive visual
failure: from about `NP3_042` and across much of the mid-orbit, SAM switches
from the bottle to the bright checkerboard/background; the bottle becomes a
hole or tiny fragment. Thus even the 19 numerical cleanups are **not usable
object masks**. Largest-component filtering cannot repair a wrong semantic
target. The tracked [visual decision](../tests/datasets/sam21_mustard_full_review.json)
binds report/sheet hashes and rejects this candidate for reconstruction.
No SfM or dense stage used any of these masks; no held-out photo or reference
mesh/depth informed selection or QA.

A separate positive-object/negative-background point-prompt candidate was
therefore scoped on frozen training views 0°, 66°, and 180°; it is documented
below as a **new trial**, not a relabeling or retry of the rejected box-only
run. No held-out/reference input or reconstruction is introduced.

## Frozen positive/negative point-prompt three-view smoke

Root visually approved original-RGB crops and exact points **before any new
inference**. The [tracked prompt manifest](../tests/datasets/sam21_mustard_point_prompts.json)
(SHA-256 `d4c8c913e831f1b78e21143d789060c0626f589baeb3f180ef2ec0526ab0c4ca`)
selects only training photos `NP3_000`, `066`, and `180`. In original 1280×1024
pixels, each has positive `(600,525)` inside the bottle and two photo-reviewed
negative points: `000` `(720,620),(700,400)`; `066` `(700,620),(750,560)`;
`180` `(720,620),(520,630)`. The box remains `[480,300,760,650]`.
The local [prompt-review crop](../.local-tools/sam21-point-review-001/prompt_review_sheet.png)
shows the chosen points on original RGB, not reference- or model-derived masks.
SAM2's [image predictor](https://github.com/facebookresearch/sam2/blob/2b90b9f5ceec907a1c18123530e92e794ad901a4/sam2/sam2_image_predictor.py)
accepts point and box coordinates in original image pixels and internally
normalizes them. The [new runner](../scripts/object_motion/sam_point_trial.py)
passes exactly those coordinates with `multimask_output=False`, retains raw
masks before acceptance, requires positive foreground and both negatives
background in raw **and** optional largest-component masks, and retains the
previous area and image-boundary guards. The same source/checkpoint hashes,
two CPU threads, available-RAM≥2.5 GiB, worker RSS≤1.5 GiB, wall≤120 s,
output≤20 MiB, and disk floor≥10 GiB apply. Nine focused SAM tests passed.

The one fresh VPS run at
`/mnt/storage/crispmesh-data/sam21-mustard-point-smoke-001/` completed in
34.52 s, peak worker RSS 1,033,008 KiB. Its manifest SHA-256 is
`09bdd382bc23d26d9306f1e0698f55ad6b8ce7fb92f230ac1a8acc982f2c38ae`.
All three raw masks obeyed point memberships `[1,0,0]`, formed a single
8-connected component, and lost zero pixels to cleanup; areas were 20,408,
16,674, and 19,642 pixels. The [three-view RGB/raw/clean review sheet](../.local-tools/sam21-mustard-point-review-001/point_review_sheet.jpg)
has SHA-256 `0c2cedc235dab3bef33f6e49815e1dca4d30525edc1c9c3985b829cc0df0d1fc`.
Initial inspection shows bottle body/labels retained and checkerboard
suppressed in these three. This is **smoke evidence only**: mask status remains
`complete_unreviewed`; it does not establish full-orbit quality. Root later
accepted exactly this three-view smoke as coarse support, recorded in the
[tracked narrow QA decision](../tests/datasets/sam21_mustard_point_smoke_review.json).
The rejected box-only 32-view
candidate remains rejected and is not retroactively reclassified.

## Common-point 48-view attempt: partial 16 sealed views

Root independently reviewed all 48 **original training RGB** crops before
inference. The proposed common point `(600,525)` lay visibly inside the bottle
in every crop; negatives `(720,620)` and `(520,630)` lay off-object. The four
12-view annotated sheets and the generated prompt file are preserved locally
at `.local-tools/sam21-mustard-point-review48-001/`. The exact
[tracked full prompt manifest](../tests/datasets/sam21_mustard_point48_prompts.json)
has SHA-256 `28440795d5f38590df8c5568969970d6623fbe22ea6b3e8781cba10608149d32`.
This is a **new common-point candidate**, not the same negatives as the
three-view smoke, and its prompt freeze preceded the larger inference run.

The [separate full runner](../scripts/object_motion/sam_point_full.py) fixed six
sequential batches of eight training photos, ≤90 s and ≤1.5 GiB worker RSS
per batch, two CPU threads, ≤600 s overall including at most 60 s aggregate
wait for ≥2.5 GiB available RAM before each batch, ≤60 MiB output, and a
≥10 GiB free-space floor. It preserves raw masks before membership, area,
boundary, and optional largest-component checks. Incomplete/unusable views
are never treated as approved masks. The reviewed model/source, package,
prompt and helper hashes are rebound throughout. The runner and synthetic
resource-exit guard tests passed independent peer review before launch.

The fresh VPS run at `/mnt/storage/crispmesh-data/sam21-mustard-point-full-001/`
stopped at its **third batch's 90-second wall cap**, not at a RAM limit.
Batch 0 sealed eight views in 58.33 s (peak RSS 1,096,116 KiB); batch 1 sealed
eight in 63.91 s (peak RSS 1,068,244 KiB). All 16 sealed masks passed exact
positive/negative membership, old area/boundary guards and optional
largest-component cleanup. Their raw areas spanned 13,892–21,432 pixels;
cleanup removed 972, 553 and 89 detached pixels from `NP3_000`, `006` and
`012`, respectively, and zero from the other 13. Batch 2 was terminated at
90.23 s (peak RSS 1,085,936 KiB) before writing a batch manifest. Six raw/
clean PNG pairs remain in that directory for failure diagnosis but are
**unsealed and unusable** as candidate evidence. There was no retry, prompt
change or resource-cap relaxation. The parent report is
`failed_or_partial_unusable`, SHA-256
`0ff3c6203301988890ded40276c58d510582bb08a7fa2f1884d2ba792349fd09`;
total elapsed 219.53 s. Batches 0/1 have manifest SHA-256 values
`ec40a87c334f1168da4e56c54dd7373dd8650bd4b71a4bdcf70a124b580436c8`
and `2f0c8d51ee40e14c3dd57679d868fae01167563479abde29fbfc262bce839ee0`.

The [RGB](../.local-tools/sam21-mustard-point-full-review-001/training_rgb_sheet.jpg),
[raw](../.local-tools/sam21-mustard-point-full-review-001/training_raw_sheet.jpg),
and [cleaned](../.local-tools/sam21-mustard-point-full-review-001/training_cleaned_sheet.jpg)
48-tile QA sheets show all training RGB crops; the mask sheets show only the
16 sealed views, with the remaining 32 mask tiles marked missing. Their hashes are
`76842c8f032e61047f16337df79e90d89cac09d3877e07bcfd9f314d792f2d41`,
`881971b008da163c575905769263379ed3fd62ec4f880a53e4f032194c3fe5cd`,
and `74f21d067eefaa80df6059999787f29566da463d0410e7f57436823d9ff8459b`.
Root's [tracked visual decision](../tests/datasets/sam21_mustard_point_full_partial_review.json)
accepts these first 16 only as **partial coarse pose-support evidence**:
bottle body, cap and labels are retained, detached background specks removed,
and no semantic target switch visible in those panels. A thin pale boundary
remains. There is no full 48-view mask set, no exact-silhouette certification,
and no SfM/dense reconstruction used these masks. Held-out photos and
reference mesh/depth were not accessed for inference or QA.

## Four-view resumable continuation: prepared, inference gated

The [new continuation runner](../scripts/object_motion/sam_point_resume.py)
keeps the same exact 48 training RGB photos, full-view common-point prompts,
SAM2 source/checkpoint, original-pixel box and mask acceptance rules. It does
not read supplied masks, depth, poses or reference geometry. It imports **only**
the prior hash-sealed first 16 masks, verifying the parent and both eight-view
batch manifest SHA-256s above, each source photo, each prompt and both PNG
hashes. The unsealed third-batch PNGs are excluded. A separate contract binds
the package, prompt, model, source inventory and helper code hashes.

New work is split into sequential four-photo CPU batches with ≤90 s per batch,
≤600 s per invocation including at most 60 s aggregate wait for ≥2.5 GiB
available RAM, ≤1.5 GiB worker RSS, two threads, ≤60 MiB output and ≥10 GiB
free space. A frame is published with raw mask, cleaned mask, provenance and
an independent hash receipt; publication can recover after interruption, and
later invocations compare all previously reported per-frame hashes. Duplicate,
extra/held-out, linked, altered or mismatched frames are rejected. Once and
only once all 48 have individually validated records, the runner can write
`complete_inventory.json` with status `generated_unreviewed` and relative
`cleaned_mask_path` values. That status is **not** visual QA or permission to
reconstruct; a separate root-authored full-set QA decision is required.

Six focused tests cover fake predictions interrupted after two published
frames, atomic receipt recovery, prior-frame and later-frame tampering, extra
held-out frame rejection, and 48-row inventory formation. The pinned Python
3.11 local test run passed 6/6. Read-only VPS preflight with the deployed
runner SHA-256 `5e9e8d0621eee892b89b27242c2bde8b4d59b23c110ddd73e9b0c25a0d419818`
validated exactly 48 frozen training rows and 16 sealed prior masks
(`NP3_000` through `NP3_108`), with package SHA-256
`45b6ed1430627a747eaeb8d4dff7d2c49502083cca0a7914e25aacd45b594425`.
At that preflight, `/proc/meminfo` reported 1,900,824 KiB available RAM, below
the 2,621,440 KiB inference gate. **No continuation inference was launched**
at that point. The proposed fresh VPS output is
`/mnt/storage/crispmesh-data/sam21-mustard-point-resume-001/`; it remains
nonexistent until a passing immediate RAM preflight. Root subsequently
approved exactly one bounded invocation conditional on the gate. The single
immediate launch check returned only 1,301,820 KiB, so the runner was **not
launched**; there was no wait, retry, mask generation, visual QA, or SfM use.

### Next inference host: M1 or Kaggle, not the VPS

The VPS launch above is historical blocked evidence, **not** a future
inference plan. Per the revised compute policy, VPS work is limited to
smaller CPU tasks; any further SAM2 inference should be evaluated on the
local M1 or Kaggle. A narrow M1 setup and three-view parity run subsequently
completed; no Kaggle run occurred.

Read-only M1 preflight found native arm64 Python 3.11.1 and 16 GiB unified
memory. The existing COLMAP virtual environment has NumPy but not PyTorch or
the SAM2 runtime. At inspection, the external `/Volumes/backups` volume had
20.86 GiB free and the internal volume only 10.47 GiB free; the latter leaves
little margin above the 10 GiB reserve. Any approved setup must place its
isolated environment, pip/cache/temp files, pinned source, checkpoint and
outputs entirely on external storage, with a ≤2 GiB setup cap and ≥10 GiB
free on **both** volumes throughout. Intended locations are
`/Volumes/backups/ai/crispmesh-sam21-m1-002` for the fresh environment/model setup attempt and
`/Volumes/backups/code/crispmesh-data` for artifacts. The pinned checkpoint is
156,008,466 bytes; the VPS source tree is about 120 MiB and runtime additions
about 10 MiB. [PyPI lists a CPython 3.11 macOS arm64 PyTorch 2.7.0 wheel](https://pypi.org/project/torch/2.7.0/)
at 68.6 MB; PyTorch's [version-pair guidance](https://docs.pytorch.org/get-started/previous-versions/)
matches PyTorch 2.7.0 with TorchVision 0.22.0, whose macOS arm64 wheel is
approximately 1.9 MB. These are download sizes, not installed-size or runtime
proof. Meta's [SAM2 setup requirements](https://github.com/facebookresearch/sam2/blob/main/setup.py)
include matching PyTorch/TorchVision plus Hydra, iopath, Pillow and NumPy;
the project installation guide primarily documents Linux, so macOS operation
was a feasibility hypothesis at preflight, not established support then.

The frozen VPS supervisor's RAM/RSS probes read `/proc/meminfo` and
`/proc/<pid>/status`; they do not work on macOS. An approved M1 lane needs a
**new**, tested Mac-specific supervisor (a conservative `vm_stat` free+inactive
memory gate and `ps -o rss=` worker probe) while keeping the old runner and its
provenance untouched. CPU inference precedes any MPS experiment so a
backend change does not confound mask parity. A bounded predeclared three-view
parity check on training frames `NP3_000`, `NP3_066`, and `NP3_108` compared
the same source, checkpoint, prompts and box against the corresponding sealed
VPS first-16 masks. Point membership, raw/clean support area and overlap were
reported; cross-platform pixel identity was not assumed before the run. No
held-out photo, reference mesh/depth or reconstruction score informed it.

The user subsequently clarified the compute policy: **future inference belongs
on the Mac M1 or Kaggle, not the VPS**. The VPS is for smaller CPU tasks and
storage. The proposed VPS continuation above is historical and superseded;
do not launch it merely because VPS RAM later recovers. M1 execution needs a
separate bounded environment/resource adapter and a cross-host mask comparison
before continuing the remaining training views. Keep weights and environment
on the external AI volume, outputs on the external code volume, and reserve
at least 10 GiB on the internal disk.

The approved external-only Mac setup `002` completed (report SHA-256
`2c520f3e07729ea881707b13fff234fd5400e851acbe1390b743abcec07fc874`);
setup `001` remained a preserved pre-install inventory-order failure. The
single frozen-source CPU parity run finished in 15.61 s, peak RSS 1,222,672
KiB, under unchanged 180 s/1.5 GiB/20 MiB and 10 GiB disk bounds. Its
[manifest](</Volumes/backups/code/crispmesh-data/sam21-m1-parity-001/manifest.json>)
SHA-256 is `bc99129f36654888b9b9fdbd7f9ac606b8527518d91424ca48e7283ac9f42741`;
the supervisor SHA-256 is
`dc82764bb2363aa41597af7280ec3462d85e14e20dae4dbdd7e667b8f4b2cc86`.
For all three selected training frames, the six decoded M1 raw/clean mask
arrays are **pixel-identical** to sealed VPS arrays (IoU 1.0, membership
`[1,0,0]`), although encoded PNG file hashes differ. The [RGB/raw/clean sheet](</Volumes/backups/code/crispmesh-data/sam21-m1-parity-review-001/review_sheet.jpg>)
SHA-256 is `f5d84f9c09b6e7a51925572b400c549ff21264e5c69d2ee0824f2eb5dfaf7e4d`.
Root independently reviewed the sheet and decoded pairs, accepting only the
three-view M1 CPU parity/coarse-support evidence in the [tracked QA record](../tests/datasets/sam21_mustard_m1_parity_review.json).
This is **not** a full-48 mask set or exact silhouette approval:
`full_training_package_ready=false`; no remaining-32 inference or SfM/dense
use occurred.
