# Mustard learned-match preflight (proposal; no model run)

## Decision and exact candidate

Trial SIFT + LightGlue on the sealed 48 TRAIN photographs. Pin inference source
to `cvg/LightGlue` commit `eb42fee2d71449efb0aa5c10549752b5d75384d8`
(read-only `git ls-remote` on 2026-09-27), and the upstream
`v0.1_arxiv/sift_lightglue.pth` release asset. The GitHub release API reports
47,632,573 bytes for this asset; **its SHA-256 has not yet been measured**.
Record its URL, size and hash after an authorized, bounded fetch. Never let
`LightGlue(features="sift")` silently fetch into the default torch cache:
upstream's constructor uses `torch.hub.load_state_dict_from_url`. Load the
locally pinned file, check state keys and shapes, and fail on any mismatch.
[Upstream model implementation](https://github.com/cvg/LightGlue/blob/eb42fee2d71449efb0aa5c10549752b5d75384d8/lightglue/lightglue.py),
[upstream release](https://github.com/cvg/LightGlue/releases/tag/v0.1_arxiv),
[upstream README](https://github.com/cvg/LightGlue/blob/eb42fee2d71449efb0aa5c10549752b5d75384d8/README.md).

The source repository carries Apache-2.0, including notice conditions. The
release hosts the weights, but the checked primary pages do not separately
state a license for the weight file. Treat the weight's commercial distribution
rights as **unresolved**, not inherited automatically from the code license.
Inventory all actual packaged transitive dependencies and their licenses before
shipping; the source requirements are lower bounds (`torch`, `torchvision`,
`numpy`, `opencv-python`, `matplotlib`, `kornia`) rather than a locked,
distribution-reviewed set. Keep OpenMVS AGPL solely in the evaluation lane.
[Upstream LICENSE](https://github.com/cvg/LightGlue/blob/eb42fee2d71449efb0aa5c10549752b5d75384d8/LICENSE),
[requirements](https://github.com/cvg/LightGlue/blob/eb42fee2d71449efb0aa5c10549752b5d75384d8/requirements.txt).

## Controlled experiment

The sealed PyCOLMAP 3.11.1 baseline uses a `SIMPLE_RADIAL` shared camera,
fixed initial intrinsics, CPU SIFT capped at 1,800 features/1,200 image size,
and exhaustive matching over 1,128 pairs. Do **not** feed its database
descriptors to LightGlue: they are quantized and cannot recover the floating
descriptors required by the upstream SIFT path. PyCOLMAP 3.11.1's direct
`Sift.extract` API does return float32 `[N,128]` descriptors and float32
`[N,4]` detections, but running the exact upstream `SIFT(backend="pycolmap_cpu")`
would require PyCOLMAP, OpenCV, Torch and Kornia in one import-compatible
process. The two existing environments split those dependencies, and the
Torch environment uses NumPy 2.4 versus the PyCOLMAP environment's NumPy
1.26. Do not claim that manually emulating its steps in the other process is
the exact upstream implementation.

For the bounded pilot, use **one explicitly experimental OpenCV SIFT +
RootSIFT extraction implementation** in the existing Torch 2.10 Python 3.11
process, with missing OpenCV/Kornia supplied only by an external pinned
overlay. Freeze OpenCV `SIFT_create(nfeatures=1800,
contrastThreshold=0.0066667, edgeThreshold=10, nOctaveLayers=4)`, maximum
image dimension 1,200, and the reviewed feature mask in the same resized
pixel grid. Divide each 128-float descriptor by its L1 norm, clamp each
component to at least `1e-6`, take componentwise square roots, then
L2-normalize, as upstream does. Serialize a single
immutable float32 feature artifact per image with ordered `(x,y)` keypoints,
scale, angle in radians, RootSIFT descriptors, image size and source hashes.
Use the same artifact bytes and indices, with no re-extraction or conversion,
in the classical and LightGlue arms. On a small sample, assert `N <= 1800`,
shapes `[N,2]`, `[N]`, `[N]`, `[N,128]`, all finite values, descriptor L2
norms within `1e-4` of one, and mask/keypoint bounds. Separately check the
OpenCV-to-COLMAP pixel-center offset on a fixture before writing database
keypoints; record any uniform `+0.5` conversion applied to both arms. This
is a *compatibility approximation*, so compare its classical arm against the
historical COLMAP arm before interpreting the learned-matcher delta.
Retain the old COLMAP arm as the historical camera baseline.
[Upstream RootSIFT and OpenCV SIFT code](https://github.com/cvg/LightGlue/blob/eb42fee2d71449efb0aa5c10549752b5d75384d8/lightglue/sift.py).

A read-only synthetic 256-pixel OpenCV 4.10 smoke in the existing PyCOLMAP
venv produced 393 SIFT descriptors with shape `[393,128]`, float32 and finite
values; explicit RootSIFT yielded L2 norms between 0.9999989 and 0.9999992.
This checks the extraction format only, not the future Torch environment's
overlay or model compatibility.
[SIFT extractor source](https://github.com/cvg/LightGlue/blob/eb42fee2d71449efb0aa5c10549752b5d75384d8/lightglue/sift.py),
[current runner](../scripts/classical_backend/run.py),
[COLMAP database format](https://colmap.github.io/format.html#database-format).

Freeze exactly the six existing TRAIN pairs (`006/012`, `012/018`,
`012/336`, `036/126`, `030/198`, `036/222`). Use two arms on the **same
extracted feature indices and descriptors**: (A) RootSIFT Euclidean nearest
neighbor, ratio `0.8` in both directions and mutual consistency; (B) SIFT
LightGlue with its default `filter_threshold=0.1`, `depth_confidence=0.95`,
`width_confidence=0.99`. Freeze these thresholds before inference. Fit the
same independent E/H and
cheirality/parallax gates already used in the [two-view result](MUSTARD-TWO-VIEW-POSE-RESULT.md).
Report raw and verified counts, spatial support and pose sensitivity to a
single correspondence or a small deterministic leave-one-out perturbation.
Do not accept a pair solely because it has more inliers: the old `030/198`
estimate changed drastically with one added verified match. If the six-pair
check establishes materially stronger, stable long-pair correspondences,
review its sealed two-arm report before a separate full 1,128-pair/SfM trial.
Use identical pair selection,
geometric verification, mapper seed `20260927`, intrinsics policy, masks,
registration gates and trajectory diagnostics for the new-feature classical
control and LightGlue. Write matches in separate fresh databases, run
`pycolmap.verify_matches`, then map: COLMAP reconstruction consumes
`two_view_geometries`, not unverified `matches` rows.
[COLMAP database documentation](https://colmap.github.io/format.html#database-format),
[COLMAP importer documentation](https://colmap.github.io/tutorial.html#feature-matching-and-geometric-verification).

All thresholds, pair choices, stopping rules and comparison outputs must be
fixed before observing the new result. Do not load the 12 held-out photos,
reference mesh, scanner/depth or supplied camera poses for selection or
tuning. Acquisition order may label diagnostics but is not a measured angle.
Success requires independently stable long-pair geometry and a plausible
TRAIN-only camera trajectory, followed later by held-out object evidence;
48/48 registration or a smoother path alone is insufficient.

## M1 and resource gates

Local machine: 16 GiB unified memory. The existing Python 3.11 PyCOLMAP venv
has PyCOLMAP 3.11.1, OpenCV 4.10.0 and NumPy 1.26.4 but no torch or kornia;
system Python 3.14 also has no torch. A read-only import check found an
existing arm64 Python 3.11.15 environment at
`/Users/christianstrobele/miniconda3/envs/nemotron_exp/bin/python` with
Torch 2.10.0, torchvision 0.25.0, NumPy 2.4.3 and available MPS, but without
OpenCV, Kornia or PyCOLMAP. Reuse it only as an immutable inference base:
install missing, pinned packages into an external `PYTHONPATH` overlay, never
into that environment. Set `PYTHONDONTWRITEBYTECODE=1`, external cache/temp
paths and record before/after hashes of the base binaries and package lock.
For product integration, construct and license-audit a separate clean lock.
The upstream SIFT class explicitly supports `backend="pycolmap_cpu"`, so its
extraction logic does not require CUDA. It also has an OpenCV backend that
returns float descriptors before RootSIFT. However `lightglue.__init__`
eagerly imports ALIKED, which imports `torchvision.models.resnet`, and SIFT
imports Kornia. Therefore a CPU-compatible source path still needs an import
smoke on the chosen Torch/torchvision/OpenCV/Kornia combination; it has
**not** been verified on this M1. The pilot uses one OpenCV extraction process
for both matcher arms and does not mix the cached PyCOLMAP descriptors with
its output. A later exact-upstream `pycolmap_cpu` trial requires an isolated,
compatible environment and a separate frozen comparison.
[Package imports](https://github.com/cvg/LightGlue/blob/eb42fee2d71449efb0aa5c10549752b5d75384d8/lightglue/__init__.py),
[ALIKED import](https://github.com/cvg/LightGlue/blob/eb42fee2d71449efb0aa5c10549752b5d75384d8/lightglue/aliked.py),
[SIFT CPU backend](https://github.com/cvg/LightGlue/blob/eb42fee2d71449efb0aa5c10549752b5d75384d8/lightglue/sift.py).
Start with eager float32 CPU, batch size one and 1,800 keypoints. MPS is a
follow-up only after a pairwise output/parity check: PyTorch supports an MPS
device, but the LightGlue source disables width pruning on CPU/MPS and its
published speed figures are hardware-specific. Do not project CUDA timing to
this M1. Avoid `torch.compile`, FlashAttention extras and mixed precision in
the first comparison. [PyTorch MPS notes](https://docs.pytorch.org/docs/main/notes/mps.html),
[LightGlue implementation](https://github.com/cvg/LightGlue/blob/eb42fee2d71449efb0aa5c10549752b5d75384d8/lightglue/lightglue.py),
[benchmark README](https://github.com/cvg/LightGlue/blob/eb42fee2d71449efb0aa5c10549752b5d75384d8/README.md#benchmark).

At preflight, the internal disk had 23,557,620 KiB free and the shared
`/Volumes/backups` device 16,615,312 KiB free (about 15.84 GiB); `/Volumes/backups/ai`
and `/Volumes/backups/code/crispmesh-data` are on that same device. Leave at
least 10 GiB free on **both** devices throughout, including package caches,
temporary files and logs. The external device has only about 5.84 GiB of
headroom above that floor. Fresh paths and caps on that **shared** external
device: `/Volumes/backups/ai/crispmesh-lightglue-sift-001/overlay` plus
`cache`, **512 MiB combined** for Kornia/OpenCV/LightGlue code and their
download caches, with no new Torch wheel; the same root's
`weights/sift_lightglue.pth`, **64 MiB** (release asset about 45.4 MiB);
`/Volumes/backups/code/crispmesh-data/mustard-lightglue-six-pair-001`,
**512 MiB** including both arms, extracted features and logs; a later
`mustard-lightglue-full-001`, **1.5 GiB** only after six-pair review. Total
reservation is under 2.6 GiB, leaving over 3 GiB above the 10 GiB floor at
the measured free space. Route `PIP_CACHE_DIR`, `TORCH_HOME`, `TMPDIR`,
`XDG_CACHE_HOME` and model file to those paths. Require a fresh
free-space check plus 1 GiB margin before each step. Cap pilot at 4 GiB
resident and 10 minutes; infer full-run deadline from measured median and
slowest pilot pair, rather than promising a GPU-derived runtime. Stop before
any output reaches its cap; preserve failed arms. Use only fresh paths under
the two designated external directories, with hashes of all sealed sources.

## Bounded M1 six-pair result

Root approved the evaluation-only pilot after this preflight. The isolated
external overlay used `opencv-python-headless==4.13.0.92`, `kornia==0.8.2`
and `kornia_rs==0.1.10`, with the pre-existing Torch 2.10.0 and torchvision
0.25.0 environment left unmodified. OpenCV 4.12.0.88 was rejected during
dependency preflight because its published NumPy requirement is `<2.3`,
whereas the existing environment has NumPy 2.4.3; OpenCV 4.13.0.92 allows
NumPy 2.4. [4.13 release metadata](https://pypi.org/project/opencv-python-headless/4.13.0.92/),
[Kornia release metadata](https://pypi.org/project/kornia/0.8.2/),
[Kornia Rust source license](https://github.com/kornia/kornia-rs/blob/main/LICENSE).

The pinned SIFT weight is 47,632,573 bytes, SHA-256
`5b52b8d9982d43532dc042606b346bb9594c9f5a4bd6f64362c63866287b4ac0`.
It loaded from the explicit external path with no unexpected keys. The only
missing state key was `confidence_thresholds`, a source-calculated buffer;
the [pilot runner](../scripts/object_motion/mustard_lightglue_six_pair.py)
asserts that exact exception. The upstream release still does not establish
separate commercial distribution rights for this weight, so it remains
**evaluation-only**.

The fresh [external report](/Volumes/backups/code/crispmesh-data/mustard-lightglue-six-pair-001/report.json)
completed in 3.50 seconds; its SHA-256 is
`57945c2a70bb71a4dba15b7b51bde6e1c9cbbc4e934f747291f9c7a7e41c2797`.
The nine TRAIN images each produced one serialized float RootSIFT artifact,
reopened unchanged for both matcher arms. The runner SHA-256 was
`4705831bdede54913461ee5b0e58c9c6b8a9eaf26ccb28ab71edd3304cd492ce`;
the reused two-view estimator SHA-256 was
`0ae09a5b5ea23b852a3166d42ab84ad0670d0255195f791e7cd2619bec135e05`.
After the report was sealed, the runner gained focused validation helpers and
tests; its current source is not byte-identical to the run version. The
external report and nine feature artifacts were not modified. All six focused
unit tests passed in the pinned external overlay, covering RootSIFT norms,
artifact hashes, the TRAIN panel seal, match indices, row-order sensitivity,
and local-only model loading.
The evaluation test module now skips those six tests during ordinary CI
discovery when its optional Torch/LightGlue dependencies are absent; the
local lean PyCOLMAP environment discovered 150 object-motion tests with six
expected skips and no import error. The external overlay still runs all six.
These counts are **raw tentative matcher outputs** passed to an independent
OpenCV E/H RANSAC fit. They have not been COLMAP geometrically verified; the
reused estimator's JSON field `verified_correspondences` names its input count
and should be read here as *tentative correspondences*.

| Frozen pair | Same-feature NN + ratio/mutual | LightGlue | LightGlue pose gate across as-returned / canonical / reverse order |
| --- | ---: | ---: | --- |
| 006/012 | 113 | 145 | distinct 53.7° / distinct 53.7° / unavailable |
| 012/018 | 127 | 151 | unavailable / unavailable / unavailable |
| 012/336 | 38 | 85 | distinct 50.7° / distinct 50.7° / distinct 31.1° |
| 036/126 | 8 | 11 | unavailable / unavailable / unavailable |
| 030/198 | 15 | 22 | distinct 129.2° / distinct 129.2° / unavailable |
| 036/222 | 13 | 44 | unavailable / unavailable / unavailable |

LightGlue found more tentative matches on all six pairs, but the opposing
`030/198` pose disappears when the same rows are reversed, and `036/222`
remains unavailable in every order. Even the middle `012/336` angle shifts
by 19.6° on reversal. The frozen long-pair reliability criterion therefore
fails. This pilot does **not** justify a full 1,128-pair/SfM run or a camera
recovery claim. The low feature count at `NP3_126` (50) also limits the
masked OpenCV approximation. At run completion, the external output was
580 KiB, the shared AI staging was 290 MiB, and both devices remained above
10 GiB free. No held-out view, scan, depth or supplied pose entered the run.
