# R03a: real stereo baseline

## Decision

Keep OpenCV StereoSGBM as a test-only baseline, not a selected production dense engine. The first actual photographed scene exposes substantial missing/incorrect disparities. It does not establish rotating-object reconstruction, object-mask propagation through matching, normals, fusion or meshing. The production CLI's full reconstruction capability remains unavailable.

An independent GPL stereo engine, libELAS, now runs as a separate local test executable. A project-authored Census matcher supplies another algorithmic comparison, but is not an independent external oracle. Both feed Middlebury-convention PFM predictions into the evaluator; neither is linked into the application. Four measured-reference scenes (Piano, Cones, Teddy and Venus) are available with pinned mirror provenance. See [ORACLES.md](ORACLES.md) for the common-input matrix, exact ranges, build commands and licensing qualifications. The user permits separate GPL test oracles; [TESTING.md](TESTING.md) defines their boundary from shipped components.

## Reproduce

Build with `CRISPMESH_WITH_OPENCV=ON` and `CRISPMESH_BUILD_TESTS=ON` as described in the root README. The evaluator is a separate target and is not linked into the core or desktop application. Dataset fetches are never triggered by ordinary CMake/CTest runs.

```sh
python3 scripts/fetch_test_data.py --fetch
python3 scripts/fetch_test_data.py
ctest --test-dir build-opencv --output-on-failure
./build-opencv/bin/crispmesh_stereo_eval \
  --left .local-tools/test-data/middlebury-2014/Piano-perfect/im0.png \
  --right .local-tools/test-data/middlebury-2014/Piano-perfect/im1.png \
  --calib .local-tools/test-data/middlebury-2014/Piano-perfect/calib.txt \
  --gt .local-tools/test-data/middlebury-2014/Piano-perfect/disp0.pfm \
  --downsample 4 \
  --output-dir build-opencv/evaluation-piano-new
```

Choose a fresh output directory for each run; existing artifacts are not overwritten. Outputs are `metrics.json`, `disparity.pfm`, and calibrated `depth.pfm`. Depth is in the calibration baseline's units (millimetres for this dataset). `--mask`, if supplied, affects **evaluation only**, not stereo matching.

For repeatable multi-engine comparison, `--prepare-only yes` writes the common grayscale PGM pair, scaled reference PFM, optional evaluation mask and provenance manifest. Pixel-only scenes accept `--ndisp` without fabricated calibration and do not produce metric depth. The standalone preparation regression checks byte-identical direct/prepared SGBM predictions, masks, missing calibration and rejected malformed/overwritten inputs:

```sh
TMPDIR="$PWD/.local-tools/tmp" python3 core/evaluation/prepared_smoke.py \
  build-opencv/bin/crispmesh_stereo_eval .local-tools/tmp
```

The full 2820×1920 input is reduced to 705×480: image pixels use area resampling; reference disparities use nearest-neighbour samples and are divided by four. Focal length and principal-point disparity offset are likewise divided by four; baseline is unchanged. This is a locally derived resolution, **not** the official quarter-resolution leaderboard protocol. Errors below are in pixels at the derived resolution. No non-occlusion mask is applied. Do not compare these numbers directly with published leaderboard scores.

Input files total 33,689,812 bytes. The official server timed out; a commit-pinned mirror supplied the files. Hashes and provenance are in [the dataset manifest](../tests/datasets/middlebury_piano_perfect.json); upstream byte equivalence remains unverified. See [TEST-DATA.md](TEST-DATA.md) for citations and limits.

## Supervised run, 2026-09-26

Fixed SGBM settings were chosen before the real run: min disparity 0, rounded disparity range 80, block size 5, P1 200, P2 800, uniqueness 10, prefilter cap 63, disparity consistency parameter 1, speckle window 100/range 32, SGBM mode. Inputs include published calibration/search bounds, not reference-driven parameter fitting. OpenCV 4.12.0, Apple Silicon, Release build.

| Metric | Result |
| --- | ---: |
| Valid reference pixels | 318,804 |
| Valid predictions at reference pixels | 267,352 |
| Coverage | 83.8609% |
| Error >2 px or missing, over all valid references | 31.3158% |
| Error >2 px, among matched pixels only | 18.0975% |
| Mean absolute disparity error, matched pixels | 1.5911 px |
| RMS disparity error, matched pixels | 4.0238 px |
| Matching time | 43.3 ms |
| Complete command elapsed time | 0.26 s |
| Maximum resident set size | 47,448,064 bytes |

Matching time comes from the evaluator; whole-command time/RSS from macOS `/usr/bin/time -l`. These are single warm local measurements, not performance guarantees. The supervised artifact is `build-opencv/evaluation-piano-supervised/metrics.json`. The independent agent run produced identical counts and errors. This repeatability is not an independent algorithm comparison.

The first live run found an evaluator bug: multiplying by the PFM header scale magnitude made nearly all disparities appear wrong. Middlebury explicitly uses only its sign for byte order and treats stored values as pixel disparities. We corrected the reader against the [primary format specification](https://vision.middlebury.edu/stereo/submit3/upload-format.html), not by tuning the reconstruction to improve its score, and added non-unit-header regression tests. The earlier incorrect report remains under `build-opencv/evaluation-piano-agent/`; it is not valid accuracy evidence.

## Next scoped tasks

1. **R03b — controlled rectification:** known rotating-board camera poses to rectified pairs; test camera-frame conversions, distortion and horizontal/vertical baseline cases against independent geometric expectations.
2. **R03c — masked depth and consistency:** prove that excluded board/background cannot contribute to accepted object depths, including matcher support and occlusions; add cross-view consistency and coverage reports. An evaluation mask alone does not satisfy this.
3. **R03d — backend comparison:** use additional measured scenes and a calibrated object/MVS reference dataset; compare a dedicated backend or isolated GPL oracle using identical inputs and explicit coordinate alignment. Keep separate development and acceptance scenes before parameter tuning.
4. **R03e — normals/fusion gate:** oriented object-frame cloud, resource limits and physical-scale checks before exposing production dense capability. Then proceed to R04 mesh/export.

DTU provides calibrated object views and structured-light reference clouds, making it a relevant later multi-view candidate. Its official sample bundle is 6.3 GB, so it was not downloaded in this bounded first experiment. See [DTU's dataset description](https://roboimagedata.compute.dtu.dk/?page_id=36). A measured LEGO-turntable dataset remains necessary for final product acceptance.
