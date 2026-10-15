# Bounded stereo profiles

`crispmesh_stereo_eval` retains its original SGBM behavior and metrics schema when
`--profile` is absent. The opt-in profiles use the same prepared images, rounded
disparity search count, and independent ground-truth scoring. No profile reads
the reference image while matching. All profiles are fixed before comparison.

| Profile | OpenCV mode | Block | Uniqueness | Speckle window/range | Extra filter |
| --- | --- | ---: | ---: | ---: | --- |
| baseline | SGBM | 5 | 10 | 100/32 | none; byte-identical baseline path |
| quality | SGBM | 5 | 8 | 100/32 | explicit left/right consistency, 1 px |
| fast | SGBM_3WAY | 3 | 10 | 50/32 | none |
| full | HH | 5 | 10 | 100/32 | none |

For `quality`, the right matcher searches `[-(D-1), 0]`, where `D` is the
rounded disparity count. The filter retains a left match at `(x,y)` if the
right disparity at `round(x-d_left),y` is finite and
`abs(d_left + d_right) <= 1`. Zero is valid; unsupported or inconsistent
matches become `+Infinity`. `mean_agreement_px` is an image-only confidence
proxy over retained pixels, not a calibrated probability. `input_valid`,
`retained`, and `retained_fraction` describe the filter's selectivity.

Explicit-profile metrics add `stage_ms` for decode, preprocess, match, filter,
and score, plus `estimated_working_bytes` and `process_peak_rss_bytes`.
Timings are wall-clock per process. The estimate is a conservative algorithmic
allocation bound; RSS is the measured process peak through metrics writing,
including input decode and scoring. Matching includes both directions for `quality`. Output encoding
and depth writing are outside the stages. The full-direction profile rejects
workloads whose 32 bytes per pixel-disparity estimate exceeds 1 GiB; all
profiles retain the existing image and pixel-disparity work limits.

Run a serial, warmup/repeated comparison on the four existing development
scenes, with a separate output tree:

```sh
TMPDIR="$PWD/.local-tools/tmp" python3 scripts/compare_stereo_profiles.py \
  --prepared piano=.local-tools/oracles/prepared/piano \
  --prepared cones=.local-tools/oracles/prepared/cones \
  --prepared teddy=.local-tools/oracles/prepared/teddy \
  --prepared venus=.local-tools/oracles/prepared/venus \
  --warmup 1 --repeats 5 --threads 1
```

The script records hashes, run-level metrics, host architecture, OpenCV thread
environment, requested OpenCV threads, reported pool limit, and medians in `summary.json`.
On this macOS GCD build, `getNumThreads()` reports the pool limit even when
`setNumThreads(1)` makes OpenCV's `parallel_for` run serially; the runner records
both values. Runs are serial and enforce a
10 GiB free-space reserve. `coverage` and `bad2_all_valid` must be read
together: rejecting uncertain pixels may lower the conditional error among
matches while worsening missing-inclusive error. Requested thread counts above
one on GCD do not guarantee a matching pool size; the controlled result here
uses one thread, verified by a runtime `parallel_for` caller-thread test.

## Development-scene result (2026-09-26)

Five measured repeats after one warmup per scene/profile, serial evaluator
processes, Apple arm64, requested one OpenCV worker. Values are medians;
`bad2_all_valid` counts missing pixels as bad.

| Scene | Profile | Coverage | bad2 all valid | Stereo ms |
| --- | --- | ---: | ---: | ---: |
| Cones | baseline | .7501 | .2891 | 22.9 |
| Cones | fast | .7522 | .2851 | 17.4 |
| Piano | baseline | .8386 | .3132 | 43.6 |
| Piano | fast | .8434 | .3021 | 32.6 |
| Teddy | baseline | .7457 | .3116 | 23.1 |
| Teddy | fast | .7509 | .2969 | 16.4 |
| Venus | baseline | .7735 | .2394 | 22.3 |
| Venus | fast | .7734 | .2378 | 15.7 |

`fast` improved missing-inclusive error and time on all four scenes. Venus
coverage fell by .0001, so this is not a strict three-axis Pareto improvement.
`quality` reduced matched-only error on Cones, Piano, and Teddy, but coverage
fell by 7.5–20.1 percentage points and missing-inclusive error worsened on
all four. `full` improved Piano missing-inclusive error (.2974 versus .3132)
at 74.2 versus 43.6 ms, and was worse on the other three scenes. Measured
peak process RSS was about 10–16 MiB for baseline, fast, and quality, versus
58–107 MiB for full. The allocation bound is deliberately much higher than
observed RSS; it is a reject threshold, not a prediction.

The complete trial record is
`build-opencv/stereo-profiles/run-20260926T110550Z-6277acf1/summary.json`.
An independent supervisor repeat with the same warm-up/repeat/thread settings
is `build-opencv/stereo-profiles/run-20260926T110906Z-cfca67bc/summary.json`.
It reproduced every reported error rate and showed 25–27% shorter median
matching time for `fast` across the four scenes. All four explicit-baseline
PFM predictions were also byte-identical to the earlier oracle-matrix baseline.
These four scenes are development data. A fresh real turntable capture is
needed before choosing a held-out winner or claiming a general improvement.
