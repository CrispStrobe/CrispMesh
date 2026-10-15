# Continuous integration

All workflows run on GitHub-hosted runners, without secrets, signing identities
or store uploads. Hosted runners have **no usable GPU**: everything below is a
CPU result. `--device mps` and `--device cuda` of the dense pipeline, and
anything that needs the development datasets, are not tested in CI.

| Workflow | File | Runs on | Triggered by |
| --- | --- | --- | --- |
| foundation | `.github/workflows/foundation.yml` | macOS, Linux, Windows | every push and pull request |
| dense pipeline | `.github/workflows/dense-pipeline.yml` | Linux, macOS arm64, Windows | `scripts/**`, manual |
| desktop app | `.github/workflows/desktop.yml` | Linux, macOS arm64, Windows | `apps/desktop/**`, manual |
| quality harness contracts | `.github/workflows/quality-harness.yml` | see file | selected `scripts/` and `tests/` paths |
| selected MVE CPU build | `.github/workflows/mve-selected.yml` | macOS, Linux, Windows | `scripts/mve_full/**`, manual |

## foundation: native core

The C++ core is built and tested here on all three systems; there is no
separate core workflow, because a second one would only repeat these steps.

```sh
cmake -S . -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build --config Release --parallel
ctest --test-dir build --build-config Release --output-on-failure
```

Proves: the core library, the `crispmesh` CLI and the registered CTest cases
(`core_contract`, `cli_capabilities`, `cli_geometry`, `cli_unavailable`) build
and pass with the default options on each system.

On macOS only it additionally builds the pinned OpenCV backend
(`-DCRISPMESH_WITH_OPENCV=ON`), runs the stereo evaluator regression, the
desktop typecheck/tests/bundle, the shared contract parity check, the whole
`scripts/` unittest discovery with a small pinned dependency set (NumPy, Pillow,
pycolmap, OpenCV; **no SciPy, scikit-image or Torch**, so tests that need those
skip there and run in the dense pipeline workflow instead) and
`cargo check --locked` of the Tauri shell.

Does not prove: the OpenCV backend on Linux or Windows.

## dense pipeline

Python 3.11 with `scripts/turntable_mesh/requirements-dense.txt`. On Linux and
Windows Torch comes from the CPU wheel index, so no CUDA runtime is downloaded.

1. `ruff check --select E4,E7,E9,F scripts/turntable_mesh` with Ruff pinned to 0.15.7.
   The repository has no Ruff configuration and Ruff 0.16 widened its defaults,
   so both the version and the rule set are explicit
2. unit tests: `test_multiscale_stereo`, `test_tsdf_hull_mesh`,
   `test_turntable_rig`, `test_scan_evaluate`, `test_dense_pipeline` and
   `test_engine_server`. The engine test replays the recorded run in
   `tests/fixtures/dense-run-sphere` (health, settings, run list, event paging,
   file serving, path traversal, token, refusal of a non-local bind without
   `--token`). With `CRISPMESH_TEST_ENGINE_RUN=1`, as set in CI, it also starts
   one real run on the synthetic sphere through `POST /api/runs` and follows its
   event log to `run_finished`
3. the README "Quick start": `synthetic_scene`, then `dense_pipeline --device cpu`
   on the analytic sphere, followed by a check that `pipeline.json` says
   `complete`, the mesh is closed, `events.jsonl` ends in `run_finished`
   `complete` and the live-preview side processes produced their meshes
4. artifact `dense-smoke-<os>`: `check/preview.png`, `check/result.json`,
   `pipeline.json`, `mesh/result.json`, `config.json`, `events.jsonl` and the stage logs

Proves: the whole driver (four stage processes, one interpreter) works on the
three systems on CPU, including the live-preview side processes, starting a
run from the HTTP engine, and the deadline handling of a stage and its child
processes (process group on POSIX, `taskkill /T` on Windows).

Does not prove: MPS or CUDA execution, speed, memory use at real photo sizes,
the AliceVision input stage (`dense_all_views_inputs` is not run by the smoke,
which starts from an inputs directory), or reconstruction quality on real
objects. The sphere is a plumbing test, not a quality gate.

Locally, in one environment that has all requirements:

```sh
export PYTHONPATH=$PWD
python -m pip install -r scripts/turntable_mesh/requirements-dense.txt ruff==0.15.7
ruff check --select E4,E7,E9,F scripts/turntable_mesh
python -m unittest scripts.turntable_mesh.test_multiscale_stereo scripts.turntable_mesh.test_tsdf_hull_mesh \
  scripts.turntable_mesh.test_turntable_rig scripts.turntable_mesh.test_scan_evaluate \
  scripts.turntable_mesh.test_dense_pipeline scripts.turntable_mesh.test_engine_server
python -m scripts.turntable_mesh.synthetic_scene --output /tmp/sphere
python -m scripts.turntable_mesh.dense_pipeline --inputs /tmp/sphere --output /tmp/sphere-run \
  --device cpu --set sizes=64,128 --set grid=96 --set planes=48 --set neighbours=4 --set best_of=2 \
  --set vote_neighbours=4 --set min_votes=2,2 --set crop_padding=6 --set hull_dilate=1 \
  --set windows=5,7 --set aggregates=1,1
```

With the two development environments, run the unit tests once with each
interpreter (tests whose libraries are missing skip) and give the driver
`CRISPMESH_PYTHON` and `CRISPMESH_TORCH_PYTHON` as described in
`scripts/turntable_mesh/README.md`.

## desktop app

- **web bundle** (Linux): `npm ci`, `npm test`, `npm run build`; uploads
  `apps/desktop/dist` as artifact `crispmesh-web`.
- **native** (macOS arm64, Windows, Linux): `npm ci`, `npm test`,
  `npx tauri build --no-bundle --ci -- --locked`; uploads the bare executable as
  `crispmesh-desktop-<os>-unsigned`. Linux installs WebKitGTK 4.1, GTK 3,
  libayatana-appindicator, librsvg, libxdo and OpenSSL headers first.
- **android / ios** (manual: `gh workflow run desktop.yml -f mobile=true`,
  non-blocking): `tauri android init` + `tauri android build --debug --apk
  --target aarch64` gives a debug-key APK; `tauri ios init` + `tauri ios build
  --debug --target aarch64-sim` gives a simulator build. Both built on their
  first run. Neither is distributable, and neither was installed or launched.
  See the TODO block in the workflow file.

Proves: the committed frontend and Rust shell compile and link in release mode
on the three desktop systems with the locked dependencies.

Does not prove: that the window opens or the UI works (nothing is launched),
installers (`bundle.active` is `false`, so no `.dmg`, `.msi`, `.deb` or
AppImage is produced), code signing, notarisation, or any store submission. The
uploaded executables are unsigned; macOS Gatekeeper and Windows SmartScreen will
object to them.

Locally:

```sh
cd apps/desktop
npm ci
npm test
npm run build                                  # web bundle in dist/
npx tauri build --no-bundle -- --locked        # native shell in src-tauri/target/release/
```

### Not done yet

- **GitHub Pages**: the web bundle is only an artifact. Publishing needs Pages
  enabled in the repository settings, a build with `vite build --base=/crispmesh/`
  (the default base `/` breaks asset URLs under a project path) and a deploy
  job with `pages: write` and `id-token: write`.
- **Installers**: set `bundle.active` to `true` (and choose targets), then drop
  `--no-bundle`. Signing is separate: Developer ID certificate plus notarisation
  credentials on macOS, a code-signing certificate on Windows.
- **Mobile**: see the workflow TODO. Release builds need an Android keystore
  and, for iOS, an Apple Developer team, certificate and provisioning profile.

## Studio and releases

`studio.yml` runs on changes to `apps/studio/`, the replay fixture or the engine
contract: type check, unit tests, the license check of everything the app
ships, the web build, and unbundled builds of the desktop shell on macOS,
Windows and Linux. Android and iOS simulator builds are manual jobs.

`release.yml` runs on a `v*` tag or by hand as a dry run. It builds the web
bundle, bundled desktop apps for the three systems, an Android debug APK, an
iOS simulator build and a self-contained archive of the Python pipeline (tested
from the unpacked archive), and writes checksums. A tag creates a draft
prerelease; a dry run only uploads workflow artifacts. Everything is unsigned
unless signing secrets are configured. The procedure is in
[`RELEASING.md`](RELEASING.md).

