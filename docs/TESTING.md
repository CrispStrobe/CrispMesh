# Reconstruction test strategy

## Evidence, not just successful execution

The test layers answer different questions:

| Layer | Reference | What it establishes | What it does not establish |
| --- | --- | --- | --- |
| Unit and contract tests | Explicit mathematical/format invariants and rejection cases | Units, conventions, bounds, parser behavior, error handling | Real image accuracy |
| Rendered-image integration | Known camera matrices and independently specified geometry | Production detector, masks, matching and triangulation work together | Physical scanner accuracy; the renderer and solver currently share OpenCV primitives |
| Real stereo benchmark | Published measured disparity and calibration | Dense pairwise accuracy, coverage, robustness on actual photographs | Rotating-marker poses, full multi-view fusion or final mesh accuracy |
| External differential oracle | A separately versioned engine's output | Disagreements worth investigating; regression cross-checks | Ground truth: two engines can agree and both be wrong |
| Browser live test | Actual CLI report imported into the running app | Import, interaction, validation and state lifecycle | Native WebKit/WebView2 shell behavior or connected hardware |
| Measured rig acceptance | Captured object, calibrated camera and independently measured dimensions/surface | End-to-end product evidence | All objects/materials/platforms |

Before the dense evaluation work, only the unit/contract, rendered-image and browser layers had been exercised. No real photographs with measured references and no independent reconstruction engine had been used. The sparse fixture's depth errors are against specified scene geometry, not a saved output from our own solver.

## Golden data and metrics

Keep measured references immutable and record source URL, citation, resolution, checksum, units, calibration and validity mask. Fetch real datasets explicitly into an ignored development directory; do not bundle them into the app. Dataset usage terms are separate from engine licensing. Do not fabricate marker boards or pretend that imported dataset camera poses exercise marker detection.

Saved output from our own engine is a **regression golden**, not accuracy ground truth. Numerical regression thresholds must allow cross-platform variation and remain tied to a named dataset/settings version. Do not silently refresh goldens after failures.

For stereo, report the number of valid reference pixels, valid prediction coverage, matched-pixel error, and an all-reference bad-pixel score that counts missing predictions as failures. Reporting only error on retained points can reward a nearly empty reconstruction. Do not compare millimetre depth errors from scenes at different scales as if they were interchangeable. Predictions may use published input calibration/disparity search bounds, but must not use reference disparities to tune settings or fill holes.

Measured multi-view acceptance will additionally require bidirectional surface distances, accuracy/completeness at named thresholds, scale error without scale-fitting it away, coverage by region, runtime and peak memory. Align coordinate frames explicitly; disclose every alignment degree of freedom.

## External GPL oracles

2026-09-27 update: the approved AGPL desktop/server roadmap also permits the
isolated OpenMVS experiments now recorded in [the comparison plan](PIPELINE-COMPARISON-PLAN.md).
The paragraph below describes the original narrower exception, not a prohibition
on those completed runs. Neither policy clears noncommercial data/code or App
Store distribution. The [evidence registry](../benchmarks/pipeline-evidence.json)
distinguishes executed pipelines from candidates and golden types from software
agreement; it is not a license approval or quality ranking.

The user allows GPL test oracles. They may be separate, opt-in development tools with their own source/version/license record. Do not link them into the app, copy their implementation into the core, bundle executables into installers, or make them runtime dependencies. Review each tool's actual terms and any generated content separately. GPL permission here is not blanket approval for AGPL or an external hosted service.

Use file-based inputs/outputs in isolated test directories. Record engine commit/build options, full command/settings, input hashes, stdout/stderr, output hashes and comparison metrics. Compare to measured data where available; label oracle-only comparisons as differential checks. A test-only reader for predicted disparity files permits these comparisons without incorporating an oracle engine into production.

This is an engineering boundary, not a legal guarantee. The [GNU FAQ on GPL development tools](https://www.gnu.org/licenses/gpl-faq.html#CanIUseGPLToolsForNF) explains the distinction between using a tool and incorporating its covered code/output into a program. Distribution of any separate tool must retain its own obligations.

## Next acceptance gates

Run the independent Python checks with the local fixture and built evaluator explicitly enabled:

```sh
CRISPMESH_TEST_REAL_DATA=1 CRISPMESH_TEST_STEREO_EVAL=1 python3 -m unittest discover -s scripts -p 'test_*.py'
node scripts/live_browser_test.mjs
```

Without these opt-ins, dataset-dependent and evaluator-CLI tests are skipped, not silently counted as evidence. Native CTest includes the deterministic stereo unit test whenever the optional OpenCV build is enabled. The current real-data results and reproduction commands are in [DENSE-EVALUATION.md](DENSE-EVALUATION.md).

1. Establish fixed-setting OpenCV stereo results on checksummed real stereo images and measured reference disparities; keep the evaluator test-only until its limitations are understood.
2. Compare a dedicated multi-view backend on an appropriate calibrated object dataset. Consider an independent GPL oracle when it provides useful algorithmic independence; using an additional engine is not required merely to produce a reference when measured truth already exists.
3. Validate rectification, object-mask propagation, consistency checks, normals and fusion on controlled scans before connecting dense capability to the shipping CLI.
4. Run the full scan on measured LEGO-rig captures and native target platforms. No browser or rectified-pair result substitutes for this gate.
