# Local stereo comparators

This is a reproducible, CPU-only comparison of the OpenCV SGBM evaluator,
external libELAS, and a small project-authored Census/Hamming reference matcher.
These are two independent codebases plus the reference matcher. Only libELAS is
an external oracle. None is linked into the CrispMesh application. The comparison
uses rectified stereo pairs and does not establish turntable-scan accuracy.

## Data, storage, and licensing

The four default scenes are measured Middlebury pairs. Fetching is explicit;
ordinary CI does not download them or run these comparators. Files are checked
against byte counts and SHA-256 values in `tests/datasets/`, including derived
PFM truth. See [TEST-DATA.md](TEST-DATA.md) for dataset sources and caveats.

```sh
python3 scripts/fetch_test_data.py --dataset piano --fetch
python3 scripts/fetch_test_data.py --dataset middlebury2003 --fetch
python3 scripts/fetch_test_data.py --dataset middlebury2001 --fetch
```

Everything stays in ignored `.local-tools/` or `build-opencv/`. Builds, tests,
and the matrix runner check that the output filesystem will retain at least
10 GiB free. Python scratch uses `.local-tools/tmp/`; it does not use `/tmp`.
Run `df -h .` before another large batch.

libELAS is pinned to the one-commit mirror at
`862ee4ef30a070753e9b92965855562a4482da05`, with Git archive SHA-256
`ae32ab63888994910e6813b65242a8f05ffc86e448048d6fe67dcd1eb4830a8d`.
The [ELAS build notes](../scripts/oracles/elas/README.md) record the compiler,
Rosetta runtime, parameters, GPL-3.0-or-later license, and bundled Triangle 1.6
terms. It is a separate, local research comparison; its redistribution and
commercial-use terms require separate review. The Census comparator is
project-authored [source](../scripts/oracles/census/census.cpp), needs only a
C++20 compiler, and is explicitly a simple reference matcher. Its 5×5 Census
signature, intensity cost, 3×3 box aggregation, and integer winner-take-all
selection make it a different implementation and matching method from libELAS
and OpenCV SGBM. It is not MGM or a state-of-the-art accuracy claim. The
investigated [MGM upstream](https://github.com/gfacciol/mgm) is AGPL-3.0 and
was not built or used. The investigated GPL-3.0 SGM-Census upstream has
nonportable 80-bit Census shifts into a 64-bit value and image-sized stack
arrays; it was not used in the matrix.

## Build and prepare

```sh
cmake -S . -B build-opencv -DCMAKE_BUILD_TYPE=Release -DCRISPMESH_WITH_OPENCV=ON
cmake --build build-opencv --target crispmesh_stereo_eval --parallel 4
sh scripts/oracles/elas/build.sh
python3 scripts/oracles/census/build.py
python3 -m unittest tests/oracles/census_test.py scripts/test_run_stereo_benchmarks.py
```

The evaluator exports the exact same grayscale P5 images for all engines.
Each `--output-dir` below must be fresh; the CLI refuses to overwrite prior
results. Piano uses its published calibration and is downsampled fourfold.
The uncalibrated 2003 and 2001 pairs use pixel disparities only; their fixed
96-candidate range was chosen conservatively before looking at scores. No
camera depth is inferred for those scenes. The scene names and paths are stable
from `scripts/fetch_test_data.py`.

```sh
eval=build-opencv/bin/crispmesh_stereo_eval
data=.local-tools/test-data
prep=.local-tools/oracles/prepared

"$eval" --prepare-only yes --left "$data/middlebury-2014/Piano-perfect/im0.png" --right "$data/middlebury-2014/Piano-perfect/im1.png" --gt "$data/middlebury-2014/Piano-perfect/disp0.pfm" --calib "$data/middlebury-2014/Piano-perfect/calib.txt" --downsample 4 --output-dir "$prep/piano-repro"
"$eval" --prepare-only yes --left "$data/middlebury-2003/cones/im2.png" --right "$data/middlebury-2003/cones/im6.png" --gt "$data/middlebury-2003/cones/disp0GT.pfm" --ndisp 96 --output-dir "$prep/cones-repro"
"$eval" --prepare-only yes --left "$data/middlebury-2003/teddy/im2.png" --right "$data/middlebury-2003/teddy/im6.png" --gt "$data/middlebury-2003/teddy/disp0GT.pfm" --ndisp 96 --output-dir "$prep/teddy-repro"
"$eval" --prepare-only yes --left "$data/middlebury-2001/venus/im2.png" --right "$data/middlebury-2001/venus/im6.png" --gt "$data/middlebury-2001/venus/disp0GT.pfm" --ndisp 96 --output-dir "$prep/venus-repro"
```

`inputs.json` declares prepared dimensions, source paths, effective disparity
count, and calibration where known. Prepared images and truth must remain
unchanged during the benchmark. The runner verifies their SHA-256 values and
the pinned source hashes before and after every engine. It rejects unpinned
real-data sources by default. The `--allow-unpinned` flag is for synthetic
runner tests only.

## Run the matrix

```sh
python3 scripts/run_stereo_benchmarks.py \
  --prepared piano=.local-tools/oracles/prepared/piano-repro \
  --prepared cones=.local-tools/oracles/prepared/cones-repro \
  --prepared teddy=.local-tools/oracles/prepared/teddy-repro \
  --prepared venus=.local-tools/oracles/prepared/venus-repro \
  --elas .local-tools/oracles/elas/build/elas_oracle \
  --census .local-tools/oracles/census/census \
  --timeout-seconds 300
```

The runner creates a fresh `build-opencv/benchmarks/run-*/` directory each
time. It records input, source, binary, adapter, and build-provenance hashes;
commands, logs, elapsed time, output hashes, score JSON, and explicit failures
in `benchmark.json`. Timeouts and nonzero exits make the run fail. Only the
evaluator receives `truth.pfm`; the two separate matcher processes receive
`left.pgm`, `right.pgm`, and the same disparity count. The raw matcher PFM is
checked for dimensions and values in `[0, count)` or `+Infinity`. Zero is a
valid disparity. SGBM rounds Piano's prepared count 65 to its required 80,
so ELAS and Census also search 0–79; the other scenes all search 0–95. SGBM's
actual `num_disparities` is checked against that common count.

## Four-scene result on this machine

The completed local report is
`build-opencv/benchmarks/run-20260926T105252Z-5527a422/benchmark.json`.
All 12 engine runs succeeded, and every engine was scored against the same
valid truth pixels within each scene. Values below are rounded from that JSON;
errors are in pixels at the evaluated image resolution. `bad2 all` counts
missing predictions as bad, whereas MAE covers matched pixels only.

| Scene (search range) | Engine | Coverage | Bad2 all valid | MAE matched |
| --- | --- | ---: | ---: | ---: |
| Piano (0–79) | SGBM | 0.839 | 0.313 | 1.591 |
| Piano (0–79) | ELAS | 1.000 | 0.184 | 1.286 |
| Piano (0–79) | Census reference | 0.981 | 0.346 | 5.167 |
| Cones (0–95) | SGBM | 0.750 | 0.289 | 0.620 |
| Cones (0–95) | ELAS | 1.000 | 0.108 | 1.052 |
| Cones (0–95) | Census reference | 0.971 | 0.197 | 4.008 |
| Teddy (0–95) | SGBM | 0.746 | 0.312 | 0.914 |
| Teddy (0–95) | ELAS | 1.000 | 0.150 | 1.262 |
| Teddy (0–95) | Census reference | 0.970 | 0.269 | 4.967 |
| Venus (0–95) | SGBM | 0.774 | 0.239 | 0.271 |
| Venus (0–95) | ELAS | 1.000 | 0.037 | 0.451 |
| Venus (0–95) | Census reference | 0.971 | 0.248 | 5.067 |

The matcher results are fixed baseline observations, not parameter search.
Piano is a calibrated stereo-depth diagnostic; the legacy scenes have only
pixel disparity. None of these pairs measures object dimensions or covers a
rotating object, so they cannot certify finished-scan accuracy.
