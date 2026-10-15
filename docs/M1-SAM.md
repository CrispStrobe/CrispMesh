# M1 SAM2 three-view parity gate

The new [Mac adapter](../scripts/object_motion/sam_m1_parity.py) is a separate,
CPU-only diagnostic for the same mustard SAM2.1 tiny candidate. It never
changes the earlier VPS reports or claims that generated masks are suitable
for reconstruction. The external-only setup `002` completed before the
one approved parity run; setup report SHA-256 is
`2c520f3e07729ea881707b13fff234fd5400e851acbe1390b743abcec07fc874`.

## Frozen three-view contract

The inputs are the exact mustard TRAIN package, the [full-48 common-point
prompt manifest](../tests/datasets/sam21_mustard_point48_prompts.json) with
SHA-256 `28440795d5f38590df8c5568969970d6623fbe22ea6b3e8781cba10608149d32`,
SAM2 source inventory SHA-256
`c7eb4585a22dadd4f54ffd9134e3103c1951745a3b4631ceb0684b55742069e3`,
and tiny checkpoint SHA-256
`7402e0d864fa82708a20fbd15bc84245c2f26dff0eb43a4b5b93452deb34be69`.
It selects `NP3_000.jpg`, `NP3_066.jpg`, and `NP3_108.jpg` only. These are
not the older three-view smoke prompts: all three use the full-48 candidate's
positive `(600,525)`, negatives `(720,620),(520,630)`, and original-pixel box
`[480,300,760,650]`. `000` is from sealed VPS batch 0; `066` and `108` are
from sealed batch 1. The adapter verifies **all 16** prior raw and cleaned
masks and both batch manifests, not just the three selected views, before
inference and again after it. The six unsealed batch-2 masks are excluded.

For each selected image, the adapter reports candidate and sealed-reference
raw/clean SHA-256, foreground pixel counts, exact-pixel equality,
intersection/union and foreground IoU, and point memberships. The engineering
parity gate is frozen before use: **raw and cleaned IoU each ≥0.99**, with
candidate and reference memberships `[1,0,0]`. Cross-platform bit identity is
reported but not required. A passing numeric result is only
`parity_observed_pending_visual_qa`; root must inspect the three outputs
before any attempt at the remaining 32 TRAIN views. No held-out image,
reference mesh/depth, supplied pose, or reconstruction metric enters this
check. A failed gate does not authorize changing prompts, threshold or model.

## Host and storage bounds

The supervisor requires native arm64 macOS, a mounted `/Volumes/backups`
device distinct from the workspace's internal APFS Data volume, and output
under `/Volumes/backups/code/crispmesh-data`. The isolated Python environment,
pip cache/temp and checkpoint must reside under
`/Volumes/backups/ai/crispmesh-sam21-m1-002`; pinned source, photos/reference,
and outputs reside on the mounted external code/data volume. Transfer/install steps remain
separately gated to ≤2 GiB setup growth and ≥10 GiB free on both volumes.
The completed setup's data staging locations are
`/Volumes/backups/code/crispmesh-data/sam21-m1-source-002` (contains `sam2/` and `LICENSE`) and
`/Volumes/backups/code/crispmesh-data/sam21-m1-three-train-002` (`photos/`,
`prior/`, `package.json`, `prompts.json`) for the three photos plus all 16
sealed reference mask pairs/manifests. Setup `001` stopped before pip
installation because its source-inventory helper sorted relative path strings
while the reviewed source digest sorted path components; all 78 local/remote
file hashes remained unchanged. Fresh `002` is the approved setup attempt,
and completed. The run used the exact staged paths above plus checkpoint
`/Volumes/backups/ai/crispmesh-sam21-m1-002/checkpoints/sam2.1_hiera_tiny.pt`
and output `/Volumes/backups/code/crispmesh-data/sam21-m1-parity-001`.

The run uses SAM2's CPU predictor with two Torch/OMP/BLAS threads, ≤180 s
worker wall time, ≤1.5 GiB worker RSS, ≤20 MiB output/log cap (log ≤1 MiB),
and ≥10 GiB free on both volumes. The Mac RAM gate is at least 2.5 GiB from
`vm_stat` **free + inactive** pages. This is a conservative operational
estimate, **not equivalent to Linux `MemAvailable`**; `ps -o rss=` provides
worker RSS. The supervisor checks limits while running and after exit, and
the worker checks essential host/resource conditions itself. Failures before
worker output creation get an exclusive sibling supervisor report. The
separate [tests](../scripts/object_motion/test_sam_m1_parity.py) cover exact
name-to-batch mapping, Darwin parser failures, external-mount/device rejection,
binary IoU, and all terminal cap boundaries; the focused pinned-Python run
passed 7/7. A rejected raw mask is saved and hash-reported before point/area
checks.

## Actual parity result

Immediately before the approved CPU-only run, read-only validation confirmed
the exact three staged training JPEGs, all 16 sealed prior mask pairs and
batch manifests, checkpoint/source pins, fresh output, 3,700,688 KiB of
free+inactive RAM estimate, and 11,112,300,544 bytes as the smaller of the
two disk-free readings. The frozen adapter source SHA-256 was
`01b159a3cf876923524f9dd8e8b0283fcbe4828d8c20a4e68a918829c92a0047`.
The single trial completed in 15.61 s with peak worker RSS 1,222,672 KiB;
the output manifest SHA-256 is
`bc99129f36654888b9b9fdbd7f9ac606b8527518d91424ca48e7283ac9f42741`
and supervisor SHA-256 is
`dc82764bb2363aa41597af7280ec3462d85e14e20dae4dbdd7e667b8f4b2cc86`.
Both volumes remained above the 10 GiB floor. No retry, MPS variant or
remaining-32 inference was run.

For `NP3_000`, `NP3_066` and `NP3_108`, **all six decoded raw/cleaned mask
arrays were pixel-identical** to the corresponding sealed VPS arrays (IoU
1.0; point membership `[1,0,0]` in each). The PNG **file hashes differ**,
so this is pixel parity, not byte/encoding parity; the reason for different
encoded bytes was not established. Cleanup removed 972 detached raw pixels
in `000`, and zero in the other two. The [hash-bound RGB/raw/clean sheet](</Volumes/backups/code/crispmesh-data/sam21-m1-parity-review-001/review_sheet.jpg>)
is 93,795 bytes, SHA-256
`f5d84f9c09b6e7a51925572b400c549ff21264e5c69d2ee0824f2eb5dfaf7e4d`.
The machine status remains `parity_observed_pending_visual_qa`. Root
independently viewed the sheet and decoded all six candidate/reference PNG
pairs, confirming pixel equality; the [tracked narrow QA decision](../tests/datasets/sam21_mustard_m1_parity_review.json)
accepts **only these three** as M1 CPU parity/coarse-support evidence. It
does not certify exact silhouettes or a complete 48-view mask package.
`full_training_package_ready=false`; the missing 32 views were not inferred
and no SfM/dense pipeline consumed these masks.

## Bounded 32-view continuation

The [separate Mac continuation](../scripts/object_motion/sam_m1_resume.py)
reuses the shared hash-receipt and inventory core with a narrowly explicit
Mac producer SHA parameter. It does not alter the sealed 16 VPS frames. Each
new frame declares the Mac runner and resource-adapter SHA-256, its CPU host
class and exact contract hash. On resume, the shared validator checks the
receipts and all previous invocation hash seals; extra/held-out or altered
frames fail closed. The contract requires the tracked accepted three-view QA
record, its setup/parity/sheet hashes, the sealed prior manifests/masks, and
the independently staged [48-TRAIN report](</Volumes/backups/code/crispmesh-data/sam21-m1-train48-001/stage-report.json>).
The stage report SHA-256 is
`d6e72143fc02ee3d18dd8961ae508417bf122cbdbc24ebc4465e207a30997861`;
all 48 photo names and source hashes are rechecked from the frozen package.
No held-out photo, supplied mask/depth/pose or reference geometry is input.

The continuation freezes four photos per batch, ≤90 s each, ≤600 s total
including at most 60 s aggregate RAM wait, CPU/two threads, ≤1.5 GiB worker
RSS, ≥2.5 GiB Mac free+inactive estimate, ≤60 MiB output, ≤1 MiB logs and
≥10 GiB free on both internal/external volumes. Generated masks remain
`generated_unreviewed`, even if all 48 are present; an independent root
full-orbit visual QA decision is required before reconstruction. The focused
[tests](../scripts/object_motion/test_sam_m1_resume.py), together with the
shared resume tests, passed 9/9, including a fake interrupted batch resumed
from 22 to 48 sealed frames and a final classical-compatible inventory.
Peer review cleared the code, not the masks or a live run.

A read-only Mac preflight verified the 48 staged TRAIN photos, the exact 16
sealed prior masks, accepted parity QA, source/checkpoint pins, and fresh
proposed output
`/Volumes/backups/code/crispmesh-data/sam21-mustard-m1-resume-001`.
The frozen Mac wrapper SHA-256 is
`5a73121d54e6da121e5632f0a84e2159809c77950c2bf780c4717d54ef86c4ea`;
shared core SHA-256 is
`c8f09966e067a6fac5d4386ed2cc044377fdb15b3456aab0987c1ebf9df38999`.
At the first read-only preflight, Mac free+inactive estimate was 3,272,688
KiB and the smaller disk-free reading was 10,860,130,304 bytes—only about
123 MB above the 10 GiB floor. Internal-disk relocation and fresh root
approval preceded the actual run.

### Actual two-invocation run and visual rejection

After relocation restored disk headroom, root approved one fresh invocation
`001`. It imported the exact 16 sealed VPS masks and produced seven new Mac
frames before a second batch tripped the unchanged **2.5 GiB free+inactive
memory gate**. The supervisor stopped the worker; `001` is preserved as
`failed_or_partial` after 32.93 s, with 23 atomic frame receipts and no
complete inventory. The invocation report SHA-256 is
`1b4dda71172611e865b83392f6cb8b09729eae80e1bd3bfbadf84c184a3e713e`.
The four-view first batch completed in 16.44 s at peak RSS 1,352,656 KiB;
the interrupted second batch had produced three valid frames before the RAM
stop. No cap was relaxed.

Root approved **one** resume `002` only after a stricter one-time launch
check of ≥4 GiB available-memory estimate. The immediate check passed at
4,276,400 KiB, and all 23 existing receipts plus the prior invocation hash
seal were revalidated before launch. `002` completed in 91.93 s, with seven
bounded batches (maximum observed worker RSS 1,428,528 KiB) and zero RAM
wait. The [completed inventory](</Volumes/backups/code/crispmesh-data/sam21-mustard-m1-resume-001/complete_inventory.json>)
has SHA-256 `b47e8caec7d05fae7bb84ae9b3eec7d4a35b1457ca9df67a3b984cbaaeed8a98`;
invocation `002` report SHA-256 is
`91ec53c6387a4e8a604e23d3c3ac5e81accd94a5f8b1154ee5b8d3e6a8201b3e`.
Read-only validation confirmed all 48 photo names and cleaned-mask hashes,
48 receipts, the exact first 16 prior frames, 32 Mac producer records, and
both invocation seals. The inventory's machine status is
`generated_unreviewed`, not an acceptance claim.

Root independently inspected the [48-view cleaned sheet](</Volumes/backups/code/crispmesh-data/sam21-mustard-m1-resume-001/review_cleaned_48_frames_002.jpg>)
(SHA-256 `755fb40755f2c8aaca38ec7113a4a074645bc08eb59191d4ccc42fb44a0b4ca4`)
and [recorded a full-set QA rejection](../tests/datasets/sam21_mustard_m1_full_review.json)
(review record SHA-256
`4084e8e458b55c566dd138f9d4b8b853ab2c68bfc20f5f831447a0cc99e9528a`).
`NP3_318` retains
checkerboard fragments above the cap; `NP3_330`, `336`, `342`, and `348`
include large checkerboard regions connected to the bottle. This is board
leakage, not evidence of a semantic target switch. The previous three-view
M1 parity acceptance remains valid only for those three views; it does not
approve the other 45. Full-set visual readiness is false, and no SfM/dense
reconstruction has used these masks. Any follow-on negative-point prompts
for the five failed training views must be a separate frozen candidate and
reviewed before inference; these generated artifacts are preserved unchanged.

### Separate five-view board-negative candidate (prepared, not run)

The [new prompt manifest](../tests/datasets/sam21_mustard_m1_board5_prompts.json)
(SHA-256 `8cc0b6fa49d977194870ef91f10b83a793294d6b3903cb1e08643016bdc9b4f8`)
keeps the original box, positive point and two negatives, adding one negative
at original pixel `(620,330)` **only** for TRAIN views `NP3_318`, `330`,
`336`, `342` and `348`. Root and an independent reviewer inspected the five
original RGB photos: the new point is on checkerboard above the cap in each.
This is an image-only candidate prompt, not a mask quality finding; it may
still fail to exclude connected board elsewhere.

The isolated [five-view runner](../scripts/object_motion/sam_m1_board5.py)
(SHA-256 `57d6248e1c234a4ac1b5d00d2a8645594596e9a68ca104c3cad9ea61269a5c11`)
uses the same pinned CPU SAM2 source/checkpoint and a separate exact-four-point
predictor. It checks the frozen full48 inventory and rejection QA, all 48 base
cleaned-mask hashes, original and new prompts, and TRAIN-only photo hashes.
The rejected 48-mask output is never edited. A successful trial would publish
only five new raw/cleaned masks with actual four-point prompts in a new
`generated_unreviewed` manifest; a resource/quality failure preserves partial
raw evidence and cannot be composed into a new 48-view set. A separate
supervisor success report and later independent five-view visual QA are
required before the classical adapter may compose any candidate inventory.

The predeclared resource contract is one CPU/two-thread batch, ≤90 seconds
worker, ≤240 seconds total, ≤1.5 GiB worker RSS, ≥4 GiB Mac free+inactive
estimate at launch and ≥2.5 GiB during execution, ≤20 MiB output, ≤1 MiB log,
and ≥10 GiB free on both internal/external volumes. Six hermetic
[tests](../scripts/object_motion/test_sam_m1_board5.py) passed, including
fourth-negative forwarding, board-foreground rejection, baseline tamper,
partial raw-mask preservation and cap boundaries. Read-only input preflight
passed on the actual Mac bundle, but free+inactive was only 3,811,184 KiB,
below the 4,194,304 KiB launch gate at that preflight. No inference ran then.
Root approved one bounded trial conditional on that gate. The permitted
60-second observation sampled 3,664,560–3,916,176 KiB, never reaching it;
the proposed fresh output and supervisor sidecar remained absent. No retry or
resource-limit change was made.

Root later approved exactly one live run after a fresh preflight measured
6,535,536 KiB available and ≥20,269,854,720 bytes free on the smaller disk.
The fresh [board5 output](</Volumes/backups/code/crispmesh-data/sam21-mustard-m1-board5-001/manifest.json>)
is **partial/unusable**, not a five-view mask set: `NP3_318`, `330` and `348`
completed as unreviewed candidates; `NP3_336` still marked the new negative
point as foreground (`[1,0,0,1]`), and `NP3_342` exceeded the predeclared 10%
largest-component removal limit. All five raw masks and the
[review sheet](</Volumes/backups/code/crispmesh-data/sam21-mustard-m1-board5-001/review_sheet.jpg>)
are preserved; the two failed views have no cleaned masks. Manifest SHA-256 is
`1008fc7bdbaa75ac33f17f8ddf44eaa275b3e98eb21466906060ebc3300c638c`,
review-sheet SHA-256 is
`610936086ea359290567d6627d1bd0309e1038c6e7b2f644f70b1e636c98b9fc`,
and [supervisor report](</Volumes/backups/code/crispmesh-data/sam21-mustard-m1-board5-001/supervisor.json>)
SHA-256 is
`b4485b72c539d59b6b3229917f0099ceb11359cdb8c9228bd0553208989619ce`
with `status=failed`. The run took 27.78 s total with peak worker RSS
1,436,656 KiB; no resource cap was relaxed. No automatic retry, visual QA
acceptance, 48-view composition, SfM or dense reconstruction followed.

### Versioned prompt recipe for the two still-failed views

A separate [v2 recipe](../tests/datasets/sam21_mustard_m1_two_view_recipe.json)
(SHA-256 `38f2a40af084df4c177925bfcfad7b5ebe5f131388283d14d15f5e690a8d0367`)
selects only TRAIN `NP3_336` and `NP3_342`, with box
`[520,365,665,600]` and the same four original-image points. The bottle is
inside the tighter box in both original RGB photos; the three negative
points are outside the box but inside the image, which pinned SAM2 handles
as independently transformed prompt tokens. The box still overlaps some
checkerboard near the cap, so this is a candidate, not proof of exclusion.
The [versioned validator](../scripts/object_motion/sam_prompt_recipe.py)
binds per-view canonical SHA-256 of `{name, source_sha256,
box_xyxy_original_pixels, points_xy_label}`. Its v1 adapter reads the frozen
five-view prompt bytes without modifying that runner or its artifacts; v2
permits 1–16 labeled, image-valid points, requires at least one positive
inside the box, and permits negative points outside it. A required reviewed
`--recipe-sha256` binds the selected TRAIN names, box and points for each run.

The bounded [generic Mac runner](../scripts/object_motion/sam_m1_recipe_trial.py)
binds the rejected base48 inventory and exact failed/partial five-view parent
hashes and file sets. It preserves raw output on per-view mask rejection,
retains the 90 s worker/240 s total, 4 GiB launch/2.5 GiB runtime memory,
1.5 GiB RSS, 20 MiB output and dual-10 GiB disk gates, and shows the broad
`[480,300,760,650]` context in review sheets even though prediction uses the
tighter recipe box. Seven focused tests passed, including v1 compatibility,
out-of-box negatives, tamper/reordered names, parent file-set safety and a
fake two-view predictor. The source SHA-256 values at the actual run were
`814fecab6cb30fafd3abd03d320ec1d4f3584437cc2697589168deb19a4da4d7`
for the runner and
`cacf2b031f3378bf8c2b9d64daf6c38ce17bdfe2e104b20008129eba82b33aaf`
for the recipe helper.

Root approved one fresh run after read-only preflight found 6,305,024 KiB
available and 20,200,599,552 bytes free on the smaller volume. Both
generated masks passed the frozen *numeric* point/area/cleanup checks, each
with one 8-connected component and zero removed pixels. The
[manifest](</Volumes/backups/code/crispmesh-data/sam21-mustard-m1-recipe-001/manifest.json>)
SHA-256 is
`361d32f4342fa193d5bfed68289f0a44151b3768b54d2a30998dd4a6b11c6f01`;
the [supervisor](</Volumes/backups/code/crispmesh-data/sam21-mustard-m1-recipe-001/supervisor.json>)
SHA-256 is
`c9124c9d1fe921396fda801f15a7f2cb9a7022a6d1829494aad9bc12ff7e703d`
(`generated_unreviewed`, no failure reason, 8.34 s total, peak worker RSS
1,430,416 KiB). The [broad-context review sheet](</Volumes/backups/code/crispmesh-data/sam21-mustard-m1-recipe-001/review_sheet.jpg>)
SHA-256 is
`58a5c387d7204099197441282f467ad4093e3e2cf44de0d048680f99dacf85b1`.
Root subsequently inspected that broad-context RGB/raw/cleaned sheet and
[accepted exactly these two masks for coarse pose support](../tests/datasets/sam21_mustard_two_view_subset_review.json)
(QA record SHA-256
`fb26b831e340062267fbb8093c9ec7b1255212ac01523a75d130126ad188f656`).
Both retain bottle, cap and labels without visible board leakage in this
review, but a fine boundary fringe remains. Acceptance is **only** for the
named two-view subset; these are not exact silhouettes, measured geometry,
or direct SfM approval. The original full48 rejection and failed five-view
parent remain unchanged. A separately QA-accepted, hash-composed full48
inventory was required before any masked SfM arm; neither that composition
nor reconstruction had run at the time of this subset-only QA.

### Subsequent TRAIN-only sparse diagnostics

A later separately reviewed complete candidate was staged, and paired
mustard SfM experiments ran. The [tracked exact-PNG keypoint audit](../tests/evidence/mustard-keypoint-mask-support.json)
binds read-only sidecar-free database snapshots: raw extraction had 109,954
keypoints, of which 8,882 fell inside the accepted coarse pose masks;
masked extraction retained exactly those 8,882 in-mask keypoints, with
per-image equality for all 48 photographs. Both initial sparse arms failed,
so this is feature-selection evidence, not successful object-track evidence.

The final image-only fixed-initial/exhaustive sparse arm registered 48/48
TRAIN photographs with 939 points. Its independently
[hash-bound observation audit](../tests/evidence/mustard-exhaustive-track-support.json)
sampled every measured COLMAP point2D against the accepted binary PNGs:
4,815/4,815 observations landed inside the coarse masks, 753 points have
observations in at least three *distinct* cameras, and 42 tracks contain 81
extra same-image observations that were retained in the denominator. The
shared SIMPLE_RADIAL parameters `[1536,640,512,0]` are fixed initial
EXIF/heuristic values, **not independently verified calibration**. These
checks support image-derived foreground-track coverage, but neither the
coarse masks nor reprojection/registration prove camera or mesh accuracy.

Meta's [SAM2 requirements](https://github.com/facebookresearch/sam2/blob/main/setup.py)
include Python ≥3.10, PyTorch ≥2.5.1 and matching TorchVision. The pinned
VPS candidate used CPU PyTorch 2.7.0; [PyPI](https://pypi.org/project/torch/2.7.0/)
lists a 68.6 MB CPython 3.11 macOS arm64 wheel and PyTorch's
[version guidance](https://docs.pytorch.org/get-started/previous-versions/)
pairs it with TorchVision 0.22.0. Wheel availability does not prove this
model/source runs correctly on M1, which is why parity precedes larger work.
