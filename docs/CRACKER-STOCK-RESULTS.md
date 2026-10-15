# P1 cracker-box results

2026-09-27, continuing [the frozen plan](CRACKER-STOCK-PLAN.md).
Run implementation/input policy frozen at `e57b669`.

## Protocol deviation: timings are not a clean serial comparison

The masked wrapper was launched before the raw mapper had exited. Masked
feature extraction and raw mapping briefly overlapped (approximately 20 seconds).
Root paused the masked feature-worker process group at 13:17:41 UTC while
leaving its parent resource monitor active; its original ten-minute deadline
was not extended. Consequently neither the raw mapping time nor the masked
feature/wall time can support an isolated speed comparison. The worker resumed
at 13:18:57 UTC (76 seconds paused). The original reports
and logs are retained, and algorithm/input settings were not changed.

## Apple product oracle

The original 60 JPEGs produced a preview USDZ in 30.525 seconds. Sampled direct
child RSS peaked at 859,635,712 bytes; this excludes system/XPC service memory.
All input hashes remained unchanged. The USDZ is 380,651 bytes, SHA-256
`00e6e5bc15a79634ffb7b0bd42492c8df5ebe575d63b228f53f737d4f23e1ae7`.
Installed `usdchecker --arkit` returned Success, with a preceding USD plugin
registration diagnostic; `usdtree` shows one Mesh prim.

A constrained export of that exact Mesh contains 1,054 vertices and 2,092
triangles. Root inspected the neutral three-view surface preview: a roughly
box-shaped region has a conspicuous unwanted thin extension. This is **not a
clean object-quality pass**. Authored meter units are not verified metric scale.
No reference-fitted surface score is claimed and no cleanup was applied.

External evidence remains under `/Volumes/backups/code/crispmesh-data/`:
`apple-crackerbox60-preview-001/result.json` and `apple-review-001/result.json`,
with the USDZ, exported PLY and neutral preview alongside them.

## Raw stock COLMAP

Execution completed, but the surviving model contains **15/60 cameras and
53 points**, below the predeclared 54-view minimum. All saved track structural
checks pass; structural validity does not make the recovered cameras correct.
Its post-hoc Berkeley diagnostic has center RMS disagreement 0.94579 times the
reference radius and p95 orientation disagreement 168.70 degrees on the paired
15 views. This is gross disagreement, not a usable object trajectory. No dense
continuation is authorized from this arm. Recorded wall time is 278.314 seconds,
subject to the overlap caveat above; no settings or pair graph were repaired.

Exact compact reports are retained in `tests/evidence/` under
`cracker-stock-raw-001.json`, `cracker-stock-raw-camera-001.json`,
`apple-crackerbox60-preview-001.json` and `apple-crackerbox60-review-001.json`.

## Feature-masked stock COLMAP

Execution completed with **2/60 cameras and 339 points**, not a usable orbit.
Only NP3_192 and NP3_198 survive. The 54-view eligibility gate fails and fewer
than four poses preclude the named-camera similarity diagnostic. No dense stage
or shape score is produced. Exact report: `tests/evidence/cracker-stock-masked-001.json`.
Recorded wall time is 156.770 seconds, including the pause; it is not a speed
comparison with the raw arm or Apple. All source/copied image and mask hashes,
worker configuration and software hashes remain unchanged.

## What this establishes

Neither stock arm produces usable object cameras on this capture; adding coarse
foreground support alone does not fix it. Apple's independent workflow runs but
its unclean output does not establish our target quality either. This narrows
the immediate issue to camera recovery/capture assumptions before dense tuning.
It does not establish a fundamental limit of photogrammetry or of all COLMAP
configurations. The next diagnostic must explain the discrepancy with the earlier
60/60 cracker-box producer, particularly initialization and intrinsics, while
keeping these failed baselines intact.

### First-divergence audit and next diagnostic

The masked database has all 60 images connected and 1,182 verified pairs loaded
by the mapper. The first automatic seed is NP3_192/198, six degrees apart. The
mapper sees 220 and 229 existing 3D points in the adjacent candidate views but
cannot register either. Six automatic seed attempts produce no third view.
The saved two-view SIMPLE_RADIAL camera has `f=5011.34797` and `k=-5.78716`,
starting from the reader heuristic `[1536,640,512,0]`; the distortion magnitude
exceeds the configured `max_extra_param=1.0`. The prior 60-view foreground
producer started from the same heuristic but used a 30-degree seed and finished
near `f=1077.58, k=-0.00723`.

This supports unstable initialization/self-calibration as a hypothesis, not a
proven causal explanation: feature limits, matching graph and verification also
differ from the prior producer. Local verified matches are not simply absent.

The next **single diagnostic**, not executed in this batch, should reuse the
sealed stock masked features/matches and automatic seed selection, changing
only mapping intrinsic refinement to fixed initial `[1536,640,512,0]`. Those are
heuristic values, not calibration or a recovered camera solution. First verify
that the implementation actually freezes both focal length and distortion and
that the saved camera remains exact. A third registered view is an early
discriminator; 54/60 plus structural checks and camera review remains the dense
eligibility requirement. Keep the same 600-second, 4-GiB RSS, 512-MiB output and
dual disk-floor bounds, fresh output and untouched parent evidence. A further
two-view failure would show that preventing intrinsic drift alone is insufficient.
This is not authorization for automatic parameter search or geometry repair.

### 2026-09-27 follow-up to the initialization audit

For this diagnostic batch, a second mapping-only arm is declared before either
new result: retain stock intrinsic refinement but set the initial pair to
NP3_192/NP3_162 (30° nominal separation, database image IDs 33/28).
This pair comes from the earlier successful 60-view producer. Its choice is
**post hoc**, so a success would diagnose seed sensitivity, not count as an
unbiased benchmark. Both arms reuse the same sealed masked feature/match
database and original images, in separate fresh directories. No image, mask,
feature or pair graph is recomputed. Evaluate all saved models and cameras,
then the named rig cameras after mapping; do not run dense from sparse coverage
alone. The two interventions are tested separately, not combined.

### Mapping-only diagnostic results

Both arms completed from byte-identical copies of the frozen masked
correspondence database (`a91caa5b6ae7c3c50f895fe682a006190118846d0510e0c8fa7ed0a1c47d9ae7`)
and original JPEGs. They made no new features or matches. Exact reports are
`tests/evidence/cracker-fixed-auto-001.json`,
`cracker-fixed-auto-camera-001.json`, and `cracker-native-seed30-001.json`.

| Mapping intervention | Retained cameras / points | Camera result | Interpretation |
| --- | ---: | --- | --- |
| Fix heuristic `[1536,640,512,0]`; automatic pair #33/#34 | 60/60; 3,805 | Fixed camera remained exact; fitted camera-center RMS 0.9665 of rig radius, p95 orientation 175.44° | Registration count improved, but the orbit is grossly wrong. This does not qualify for dense reconstruction. |
| Native intrinsic refinement; force pair #33/#28 | 3/60; 211 | Saved `f=1235.31`, `k=1.59543`; too few views for rig Sim(3) evaluation | Wider seed by itself is insufficient with these stock features/matches. |

The first arm's structural sparse flag is `true` by design because it checks
registration and model integrity *pending* independent camera review. That
review failed decisively; quality acceptance remains false. Neither arm gets
a dense continuation. Mapping times (18.745 and 23.276 seconds) exclude prior
feature/match computation and are not complete pipeline timings.

This corrects our language about “stock COLMAP” for this dataset. Defaults
target ordinary static unstructured scenes; this is a small rotating object
against stationary background. One earlier masked, sequential, lower-feature
configuration registered 60/60 and agreed much better with evaluation rig
cameras. Changing just the pair or freezing an uncalibrated camera did not
reproduce it. A controlled turntable configuration and pose review are required
before mesh quality or product-readiness claims.

Rechecking that prior 60-view model with the **same** sealed rig-camera adapter
used above gives center RMS 0.01787 of rig radius and p95 orientation 2.987°
(`tests/evidence/cracker-prior60-camera-recheck-001.json`). This confirms a
large pose difference between the two 60-view outputs under one metric and
reference policy. It remains evaluation-only rig agreement, not physical mesh
accuracy; earlier mesh shape scores remained poor. In four sampled NP3 feature
masks, the permitted region occupies 2.8–5.0% of the 1280×1024 image, giving
self-calibration relatively little image area to constrain distortion.
