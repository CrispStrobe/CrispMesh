# CrispMesh

Photos of an object in, a closed printable mesh out. CrispMesh turns turntable
photos into a watertight STL with one native program (Rust and WebGPU) that
runs on desktop GPUs, in the browser and inside **CrispMesh**, the app
for macOS, Windows, Linux, iOS and Android.

[![dense pipeline](https://github.com/CrispStrobe/CrispMesh/actions/workflows/dense-pipeline.yml/badge.svg)](https://github.com/CrispStrobe/CrispMesh/actions/workflows/dense-pipeline.yml)
[![studio](https://github.com/CrispStrobe/CrispMesh/actions/workflows/studio.yml/badge.svg)](https://github.com/CrispStrobe/CrispMesh/actions/workflows/studio.yml)
[![foundation](https://github.com/CrispStrobe/CrispMesh/actions/workflows/foundation.yml/badge.svg)](https://github.com/CrispStrobe/CrispMesh/actions/workflows/foundation.yml)

## What is here

| Part | Where | State |
| --- | --- | --- |
| **Native pipeline**: photos to a closed STL, masks, cameras, GPU dense stages | [`crates/dense/`](crates/dense/README.md) | The primary implementation; one command, no Python, no external program; Metal, Vulkan, DirectX 12, WebGPU; library API and C interface |
| **In the browser**: the same pipeline as WebAssembly and WebGPU | [`crates/dense/web/`](crates/dense/README.md), live at [crispstrobe.github.io/CrispMesh](https://crispstrobe.github.io/CrispMesh/) | From photos or prepared inputs, threaded where the page allows; nothing is uploaded |
| **CrispMesh**: the app, one code base for desktop, phone and web | [`apps/studio/`](apps/studio/README.md) | Reconstructs in-process from photos, live progress and diagnostics, example objects; [prereleases](https://github.com/CrispStrobe/CrispMesh/releases) and internal TestFlight builds ([releasing](docs/RELEASING.md)) |
| **Engine contract**: event log, artifacts, replay bundles, HTTP engine | [`docs/ENGINE-CONTRACT.md`](docs/ENGINE-CONTRACT.md) | Shared by the native engine, the browser engine and the Python reference |
| **Python reference pipeline** and evaluation tools | [`scripts/turntable_mesh/`](scripts/turntable_mesh/README.md) | What the native crate was ported from; no longer extended. The scanner evaluator and test-scene tools stay in Python |
| **Core**: C++20 library and CLI for projects, calibration, marker boards | [`core/`](core/README.md) | Builds and tests on macOS, Linux, Windows |
| **Project workspace**: the earlier Tauri desktop app around the core | [`apps/desktop/`](apps/desktop/README.md) | Superseded by Studio for reconstruction |

Current state, results and limits: [`docs/STATUS.md`](docs/STATUS.md). The
modular plan (providers for masks and cameras in front of one dense
reconstruction): [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

## From photos to a mesh

The native program (`crates/dense`, Rust and WebGPU) goes from a folder of
turntable photos and a lens calibration to a closed STL in one command, with
no Python and no other program:

```sh
cargo build --release --manifest-path crates/dense/Cargo.toml
crispmesh-dense run --photos data/bunny/rgb \
  --calibration scripts/turntable_mesh/calibrations/3dlf-pro.json \
  --output runs/bunny
```

New `run` requests start with a silhouette surface. Add `--splats capture` for
optional Gaussian refinement (3,000 steps, crop 320, mesh 192), or select
`--base-surface stereo` for the retained stereo route. Original handheld or
irregular captures can use the app with the configured
[desktop capture host](scripts/photo_regression/README.md#automatic-real-capture-route);
that learned-camera route needs external tools on the Mac.

Masks come from a grey threshold with the contact shadow taken out, cameras
from our own turntable solver; both are the defaults, and SAM or COLMAP can
be chosen instead (`--masks`, `--cameras`). On the 3DLF Bunny (73 photos,
Apple M1 with 16 GB) the command takes about 93 seconds and the STL scores
0.964 / 0.995 / 1.000 above the support against an independent scan; the
Armadillo, Dragon and Lucy take about 83, 78 and 50 seconds. Numbers per stage are in the [crate README](crates/dense/README.md)
and the solver in [`docs/TURNTABLE-SOLVER.md`](docs/TURNTABLE-SOLVER.md).

The Python package is the reference the native program was ported from:

```sh
export PYTHONPATH=$PWD
pip install -r scripts/turntable_mesh/requirements-dense.txt

# no dataset needed: an analytic test scene, about a minute on CPU
python -m scripts.turntable_mesh.synthetic_scene --output /tmp/sphere
python -m scripts.turntable_mesh.dense_pipeline --inputs /tmp/sphere --output /tmp/sphere-run --device cpu \
  --set sizes=64,128 --set grid=96 --set planes=48 --set neighbours=4 --set best_of=2 \
  --set vote_neighbours=4 --set min_votes=2,2 --set crop_padding=6 --set hull_dilate=1 \
  --set windows=5,7 --set aggregates=1,1

# a real photo set with recovered cameras and masks
python -m scripts.turntable_mesh.dense_pipeline --scene final.sfm --prepared undistorted/ --raw-masks masks/ \
  --output runs/my-object --device mps        # or cuda, or cpu
```

The pipeline repairs masks from multi-view consensus, builds a silhouette hull,
matches all views coarse to fine, fuses depth into a signed distance volume
inside the hull and extracts a closed surface. Inputs, every setting, the
design reasons and all measurements are in the
[pipeline README](scripts/turntable_mesh/README.md).

With AliceVision and SAM 2.1 installed, the same command starts from a plain
folder of turntable photos and a lens calibration (`--photos`, `--calibration`)
and recovers masks and cameras first; see
[`docs/PHOTOS-TO-INPUTS.md`](docs/PHOTOS-TO-INPUTS.md).

## Watching a run, and the front end

Every run writes an event log and, while it is still matching, coarse preview
meshes and diagnostic sheets (mask repair, hull against masks, depth per
level, mesh outline against the photos). A front end can follow that locally,
over HTTP, or from a recorded bundle:

```sh
python -m scripts.turntable_mesh.engine_server --runs runs/ --data data/ --static apps/studio/dist
```

```sh
cd apps/studio && npm ci && npm run dev     # opens with a recorded demo run, no engine needed
```

Studio shows the stage timeline, the surface improving step by step in a 3D
viewer, the diagnostics gallery, per-step inspection sheets, numbers and
reports; on desktop and phone it reconstructs in-process, in the browser on the
visitor's GPU, and it can also drive a remote engine. See [`apps/studio/README.md`](apps/studio/README.md) and the
[engine contract](docs/ENGINE-CONTRACT.md).

## Results

The default command from photos, F1 against independent scans at 0.5 % of the
scan's diagonal (whole surface / above the support; scans used for scoring
only), 73 photos per object, Apple M1:

| Object (3DLF) | F1 at 0.5 % |
| --- | --- |
| Bunny | about 0.90 / 0.96 |
| Armadillo | about 0.92 / 0.95 |
| Dragon | about 0.80 / 0.85 |
| Lucy | about 0.85 / 0.88 |
| Thai statue | about 0.88 / 0.95 |
| Happy Buddha | about 0.79 / 0.83 |

On rendered Google Scanned Objects the rhino figurine scores 0.867 and a
cereal box 0.833 at 0.5 %; on DTU (masked) 1.2–1.5 mm. Exact numbers, other
image sets and every experiment: [`docs/STATUS.md`](docs/STATUS.md),
[`docs/OTHER-IMAGE-SETS.md`](docs/OTHER-IMAGE-SETS.md) and the
[crate README](crates/dense/README.md).

Known limits, in short:

- Threshold masks assume a dark object on a light backdrop; light-coloured
  objects need SAM (optional) or imported masks.
- Interiors a single ring of cameras never sees are capped; very smooth,
  untextured objects fail camera recovery.
- Fine relief is partly lost at the finest matching level (work in progress).
- The 3DLF scans are mirror images of the photographed objects; our results
  have the correct handedness.

## Continuous integration

GitHub Actions build and test on macOS, Linux and Windows: the Python pipeline
(lint, unit tests, an end-to-end run through the CLI and through the HTTP
engine), the C++ core, Studio (type check, tests, build) and the Tauri
project workspace. There is no GPU in CI. See [`docs/CI.md`](docs/CI.md).

## Earlier engine comparisons

The [comparative benchmark](docs/BENCHMARK-RESULTS.md) now evaluates MVE and
[COLMAP/OpenMVS](docs/CLASSICAL-BACKEND.md) on real object photographs and a
separate upstream software control. It reports surface accuracy/completeness,
normal agreement, topology, camera diagnostics, resource limits and failed runs;
software-oracle agreement is not physical ground truth.
The latest [fresh 60-photo M1 oracle](docs/TURNTABLE-COMPLETE-PLAN.md#fresh-complete-openmvs-oracle-and-shape-result)
does finish from JPEGs through a textured OpenMVS mesh, but its shape is
**rejected** (42.23% reference-fitted F1@1% scanner diagonal; 30.92% in a
camera-transported common gauge). OpenMVS is AGPL evaluation software here,
not an app-integrated or approved commercial/App Store backend. A separate
[AliceVision-Mac audit](docs/ALICEVISION-MAC-ORACLE.md) ran an isolated Metal
depth test, not a complete AliceVision reconstruction.

## Project workspace (`apps/desktop`)

Requires Node 22.18+ (Node 24 recommended) and npm.

```sh
cd apps/desktop
npm ci
npm run dev
```

Open the local URL printed by Vite. Create a project or import a manifest from `tests/contracts/`. The example calibration is synthetic, and no photographs accompany those fixtures. Image entries are relative paths, not uploaded image data. Download the project JSON to keep a portable copy.

For the native desktop shell, install the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/), then run `npm run tauri -- dev` in `apps/desktop`. The shell currently exposes capability status only; desktop worker integration is a later milestone. Installer bundling is disabled until packaging and dependency review are complete.

During initial development a Rust toolchain was installed locally in ignored `.local-tools/` without modifying the shell profile. If using that installation, run from the repository root before native commands:

```sh
export CARGO_HOME="$PWD/.local-tools/cargo"
export RUSTUP_HOME="$PWD/.local-tools/rustup"
export CARGO_TARGET_DIR="$PWD/apps/desktop/src-tauri/target"
export PATH="$CARGO_HOME/bin:$PATH"
```

## Build the CLI

Requires CMake 3.20+ and a C++20 compiler. No external native libraries are linked into the foundation.

```sh
cmake -S . -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build --config Release --parallel
ctest --test-dir build -C Release --output-on-failure
./build/crispmesh capabilities
./build/crispmesh validate tests/contracts/calibrated-project.json
./build/crispmesh diagnose-geometry
```

On Visual Studio generators the executable is `build/Release/crispmesh.exe`. The [core README](core/README.md) describes commands, return codes and the C ABI.

To enable actual image-to-pose estimation, build the pinned OpenCV configuration:

```sh
cmake -S . -B build-opencv -DCMAKE_BUILD_TYPE=Release -DCRISPMESH_WITH_OPENCV=ON
cmake --build build-opencv --config Release --parallel 4
ctest --test-dir build-opencv -C Release --output-on-failure
./build-opencv/bin/crispmesh estimate-poses build-opencv/synthetic-pose-fixture/project.json
./build-opencv/bin/crispmesh reconstruct-sparse build-opencv/synthetic-sparse-fixture/project.json
```

This test-generated dataset is explicitly synthetic. For your photographs, supply a project with matching camera calibration and measured rotating-board geometry; see [pose contracts and gates](docs/POSE-MILESTONE.md). The CLI emits a JSON report on stdout. Save it to a separate file and use **Import pose report** in the workspace to inspect translations, rotations and reprojection errors. Importing a report does not mark pipeline stages complete. The first enabled build downloads and compiles [pinned OpenCV sources](docs/OPENCV.md).

Sparse reconstruction additionally requires a matching binary grayscale PNG object mask for every image. It re-estimates poses, matches nearby views in acquisition order, and outputs millimetre-scale points with observations and residuals. Import the matching project and use **Import sparse report** to inspect the saved JSON and orbit/zoom the point cloud. The current baseline is limited to 64 views and 256 million aggregate pixels; see [sparse contracts and verification](docs/SPARSE-MILESTONE.md).

## Verify changes

```sh
npm --prefix apps/desktop ci
npm --prefix apps/desktop test
npm --prefix apps/desktop run build
node tests/contract-parity.mjs
python3 scripts/check_dependencies.py
```

The parity test needs the built CLI; pass a different executable path as its first argument when needed. The dependency check additionally requires Python 3.11+, Cargo and the committed lockfiles. CI runs native and web checks; a configured workflow is not evidence that every target has already passed.

Testing now includes a separate dense-stereo evaluator and a checksummed real Middlebury scene with measured reference disparities. See [test strategy](docs/TESTING.md), [dataset provenance](docs/TEST-DATA.md), and [measured results and reproduction](docs/DENSE-EVALUATION.md). This evaluator is not a production dense-reconstruction stage.

For the live Chromium test, use the development-only browser tooling (not shipped with the app):

```sh
npm install --prefix .local-tools/browser --save-exact playwright@1.63.0
PLAYWRIGHT_BROWSERS_PATH="$PWD/.local-tools/browsers" .local-tools/browser/node_modules/.bin/playwright install chromium
node scripts/live_browser_test.mjs
```

Run the OpenCV CTests first to generate the fixture. The browser script runs the actual sparse CLI, starts and stops its own Vite server on port 1430, and checks report import, orbit/zoom, rejection, state invalidation and narrow layout. It does not test the native webview or physical scanner.

## Architecture and next steps

The pipeline is modular: masks, cameras and undistortion are provider-based
stages in front of one native dense reconstruction, so each platform uses the
providers it can run. See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for
the plan, [`docs/STATUS.md`](docs/STATUS.md) for what is in progress and
[`docs/SURFACE-METHODS.md`](docs/SURFACE-METHODS.md) for the current comparison of
surface methods (ours, Gaussian splatting, neural surfaces, classical
pipelines) and what we build next.
Older planning documents (`docs/PLAN.md`, `docs/SOTA-ROADMAP.md`) describe the
earlier C++-core route.

## License

Original CrispMesh project code is licensed under [GNU AGPL v3 only](LICENSE) (`AGPL-3.0-only`). Third-party code, assets, and datasets retain their own licenses and attribution requirements; the project license does not relicense them. The dependency-selection policy and exact shipped dependency/source-bundle audit remain separate release gates. As sole copyright holder, the author also distributes official store builds (App Store, Mac App Store, Google Play) under the stores' terms; this additional permission under AGPL-3.0 section 7 is stated in [NOTICE](NOTICE), applies only to binaries published by the copyright holder, and does not change the AGPL rights to the source. Store builds contain no third-party GPL, LGPL or AGPL code. `apps/desktop/package.json` remains `private: true` because npm publication is unrelated to GitHub repository visibility.

Every library, engine and dataset the pipeline and the apps depend on is listed
with its license in the [pipeline README](scripts/turntable_mesh/README.md#dependencies-and-licenses)
and the [Studio README](apps/studio/README.md). A successful local build is
not a statement that a combination may be redistributed.
