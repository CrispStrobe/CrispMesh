# Full MVE CPU photo-to-mesh experiment

This is a separate local CLI experiment, not a shipping dependency or the app's `reconstruct` implementation. It accepts raw photographs with unknown camera poses. Apple Silicon runs produced real-image clouds and meshes, but the licensed bunny object's [reference-fit shape test](BUNNY-EVALUATION.md) failed badly. No object-quality, metric-scale, cross-platform runtime, or final artifact-license gate has passed.

## Selected source and build

Source: [MVE commit `bf2279f161ba962072ecac85224c15e82bc5f52e`](https://github.com/simonfuhrmann/mve/tree/bf2279f161ba962072ecac85224c15e82bc5f52e), archive SHA-256 `0e33d40b150313617d28ab4297459939e9ad59c47f21566ed649daf3b34c47f6`. `scripts/mve_full/build.sh` verifies that archive, applies [`sift_only.patch`](../scripts/mve_full/sift_only.patch), and [checks all 215 selected source files](../scripts/mve_full/verify_source.py) against the archive or exact patched-file hashes. It then builds only `math`, `util`, `mve`, `sfm`, `dmrecon`, `fssr`, and the six CLI tools listed below with GNU Make. The patch switches `sfmrecon` from `FEATURE_ALL` to `FEATURE_SIFT`, removes the SURF call from feature extraction, and excludes `surf.cc` from the selected static archive. The build verifies that `surf.o` is absent. A separate binary artifact and runtime dependency inventory is still needed for shipping. A root `make all` is unsuitable because it includes the GPL `sceneupgrade` app.

MVE's selected files carry BSD-3-Clause notices. Its [license](https://github.com/simonfuhrmann/mve/blob/bf2279f161ba962072ecac85224c15e82bc5f52e/LICENSE.txt) warns that `libs/sfm/sift.cc` and `surf.cc` may have patent or other use limits. [OpenCV's 2020 release note](https://opencv.org/opencv-4-4-0/) reports expiry of the SIFT patent; this does not, by itself, resolve every jurisdiction or MVE's broad warning. The SURF **algorithm implementation** is excluded from the selected archive and is not invoked; `surf.h` descriptor types and unused matching support remain in compiled headers/code. This is not a blanket patent or license approval. MVE's own [README](https://github.com/simonfuhrmann/mve/tree/bf2279f161ba962072ecac85224c15e82bc5f52e) calls Windows support intermittent. The selected CMake build and process runner have Windows paths, but Windows has not been built or validated. macOS linking uses locally installed JPEG, PNG and TIFF libraries, whose exact runtime dependency and redistribution inventory is not complete.

The Apple Silicon CMake executables' `otool -L` output names Homebrew `libjpeg.8.dylib`, `libpng16.16.dylib`, and `libtiff.6.dylib`, plus macOS system libraries. The installed libtiff library further links WebP, Zstandard, liblzma, JPEG and system zlib. Installed license texts identify IJG/BSD for libjpeg-turbo, PNG Reference Library v2 for libpng, LibTIFF's permissive license and bundled LZW notice, BSD-3-Clause for WebP and Zstandard, and 0BSD for **liblzma**. XZ's package also contains GPL scripts and LGPL fallback `getopt_long`; its installed `COPYING` says the linked liblzma is 0BSD. This is a local binary dependency trace and license screen, not an exact packaged artifact audit or shipping sign-off.

From the repository root, with the pinned archive in `.local-tools/mve-spike/mve-bf2279f.tar.gz`:

```sh
bash scripts/mve_full/build.sh
TMPDIR="$PWD/.local-tools/tmp" python3 -m unittest scripts.mve_full.test_run -v
```

The selected [CMake target set](../scripts/mve_full/CMakeLists.txt) also configured and compiled all six executables on Apple Silicon with CMake 4.1.1/Ninja and AppleClang 17. Its `libmve_sfm.a` contains `sift.cc.o` and no `surf.cc.o`. macOS has local real-photo execution; macOS and Linux also have passing remote build/smoke checks:

```sh
cmake -S scripts/mve_full -B .local-tools/mve-cmake-build -G Ninja \
  -DMVE_SOURCE="$PWD/.local-tools/mve-spike/mve-bf2279f161ba962072ecac85224c15e82bc5f52e" \
  -DCMAKE_BUILD_TYPE=Release
cmake --build .local-tools/mve-cmake-build --parallel 2
```

`CRISPMESH_MVE_OPENMP` is an optional CMake switch, OFF by default. Enabling it requires a working `OpenMP::OpenMP_CXX` toolchain and has not been timed or quality checked. All measurements here use the default serial MVE reconstruction path on macOS.

The scoped [GitHub Actions matrix](../.github/workflows/mve-selected.yml) targets macOS arm64, Ubuntu x64 and Windows x64. It verifies the pinned source, builds the selected tools, checks CLI startup and PNG ABI, and runs backend unit tests. The source helper preserves a 10 GiB disk reserve. macOS and Ubuntu passed in [run 36274296663](https://github.com/CrispStrobe/CrispMesh/actions/runs/36274296663). No remote job has run real photographs; compile/usage checks do not establish reconstruction quality or packaged license approval.

Windows first exposed CRLF patch handling and an outdated hard-coded Visual Studio generator; both were corrected. It then reached compilation and exposed Windows `min`/`max` macro collisions. Commit `d0d3856` adds `NOMINMAX` to the selected Windows targets without modifying pinned upstream source. The [verification retry](https://github.com/CrispStrobe/CrispMesh/actions/runs/36275102048) **did not start**: GitHub reported failed account payments or a spending-limit issue. Thus the latest Windows fix is unverified, and Windows support is **not passing**. No billing setting was changed and no further retry was launched.

The six selected commands are `makescene --images-only`, `sfmrecon`, `dmrecon`, `scene2pset --fssr=SCALE`, `fssrecon`, and `meshclean`. `scene2pset --fssr` writes the oriented normals, scale values and confidence needed by FSSR. The runner [run.py](../scripts/mve_full/run.py) copies the chosen photos into a fresh ignored output directory, hashes them and each executable, records stage arguments and logs, enforces a total time/output/log cap and a 10 GiB free-space floor, then rejects absent or empty geometry. FSSR and native `meshclean` can leave exact-zero-area faces; the [final sanitizer](../scripts/mve_full/sanitize_mesh.py) retains `mesh-native.ply`, copies every vertex and retained face record byte-for-byte to `mesh.ply`, removes only those zero-area faces, and strictly validates the result. It takes no camera-pose or board file. Its output coordinate system has an arbitrary SfM gauge; no length unit is inferred.

```sh
TMPDIR="$PWD/.local-tools/tmp" python3 -m scripts.mve_full.run \
  --images .local-tools/test-data/tree-subset/images \
  --output build-opencv/mve-full-tree-NEW \
  --max-views 10 --max-pixels 1500000 --scale 2 \
  --max-gib 2 --timeout-minutes 15
```

An optional `--image-list FILE` selects photo basenames in explicit order. `--binary-dir .local-tools/mve-cmake-build` uses the selected CMake binaries, but the runner records their provenance as unverified until an external artifact attestation is added. The output must be fresh. `--stop-after sfm` saves an early camera-only diagnostic. Runner schema v2 records its source hash and a declared minimum registered-view fraction (default 0.7; adjustable with `--min-registered-fraction`). It validates finite binary PLY coordinates, face indices, nondegenerate triangles and exact payload. Its `result.json` still sets `shipping_approved: false`, `scale_verified: false`, and `surface_quality_accepted: false`: file, point and face checks do not score surface accuracy.

## First real-photo result

The input was the ten JPEG subset of Matthew Guertin's [Single Tree Photogrammetry Dataset](https://huggingface.co/datasets/Matt1up/tree-minnetonka-photogrammetry), [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/), credited in [REAL-DATA.md](REAL-DATA.md). Only JPEGs were supplied; the source's camera export was excluded from the run. The pinned manifest is `.local-tools/test-data/tree-subset/manifest.json`. The run record and artifacts are ignored local files at [`build-opencv/mve-full-tree-001/result.json`](../build-opencv/mve-full-tree-001/result.json).

| Measure | Observed |
| --- | ---: |
| Imported photographs / estimated registered cameras | 10 / 9 |
| Oriented cloud points | 322,133 |
| Raw mesh vertices / faces | 49,375 / 97,045 |
| Cleaned mesh vertices / faces | 35,785 / 69,897 |
| End-to-end wall time | 51.15 s |
| Output bytes | 269,988,239 |

The independent supervisor parsed the complete binary PLY payload, checked finite coordinates, valid triangle indices, positive triangle areas, and found zero zero-area triangles. This supports that the artifact is a valid nonempty mesh. The tree scene contains wide background geometry and has no object masks or measured scale, so this run does not establish an isolated, accurate object scan. The previously tested masked MVE depth adaptation was not in this run; see [masked-depth findings](MVE-MASKED.md). A larger licensed object-photo capture and independent shape/coverage reference are needed to judge useful reconstruction. Similarity alignment to a reference scan could score shape if its procedure and evaluation region are frozen in advance, but it cannot recover metric scale.

The first run predates the later per-stage duration and peak-single-child-RSS additions to the runner, so `result.json` records cumulative times only. The current runner labels its RSS measurement as the largest reaped child, not total process-tree peak. No claim is made for App Store approval from this local build.

## Difficult object-photo follow-up

The licensed [3DLF-Scan bunny subset](https://data.mendeley.com/datasets/ngvgpsvd8b/1) contains 73 real turntable RGB photos and a separately acquired reference mesh. The first raw-photo run (`build-opencv/mve-full-bunny-001/result.json`) imported all 73 frames, extracted only roughly 30–80 SIFT features per image, formed 80 tracks, and stopped during initial-pair selection without camera poses or a mesh. This is a genuine pipeline failure. The supplied metric poses and reference mesh were not inputs.

The fixed [image-only preparation helper](../scripts/mve_full/prepare.py) offers named `gamma05`, `clahe2` and `gamma05_clahe2` profiles. `gamma05` applies an 8-bit channel lookup table with exponent 0.5. `clahe2` applies CLAHE to LAB lightness with clip limit 2 and an 8×8 tile grid using an existing local OpenCV Python installation. Each output PNG and source photo is hashed, sources are checked again after processing, and the profile and versions are recorded. It reads no camera poses, reference mesh or evaluation labels. A run using prepared images must be reported as its own input profile, separate from the failed raw baseline.

For the fixed `gamma05_clahe2` profile, a second run (`build-opencv/mve-full-bunny-002/result.json`) registered all 73 of 73 cameras and produced 3,286,728 oriented points and a raw FSSR surface with 853,571 vertices and 1,721,459 faces. Its original runner rejected the raw surface after 944.62 s because FSSR emitted zero-area triangles that `meshclean` was supposed to remove. This failed result and raw mesh were left unchanged. A [frozen continuation](../scripts/mve_full/finalize.py) verifies the failed result, raw mesh, selected source, meshclean binary, and all 73 copied photograph hashes before rerunning **only** `meshclean` into a fresh directory. The successful [`002-finalized-v2/result.json`](../build-opencv/mve-full-bunny-002-finalized-v2/result.json) reports 589,429 native cleaned vertices and 1,193,457 native faces. Eleven native faces have exact zero area; filtering only these produces a strictly valid 589,429-vertex, 1,193,446-face binary mesh. The continuation took 12.36 s internally (3.56 s native `meshclean`), or 16.25 s externally including provenance checks; it reuses the prior SfM, MVS and FSSR stages and **is not a fresh end-to-end success or timing**. The prepared photos themselves took 26.80 s externally in a separate repeat, with byte-identical output hashes. Photo preprocessing, reconstruction, and cleanup times must therefore be reported separately, not as a single measured one-command run. The supplied reference mesh/poses were not consumed during reconstruction. Independent [reference-fit evaluation](BUNNY-EVALUATION.md) found F-score **14.88%** against an 86.25% same-reference sampling control, with severe visible shape distortion. This rejects the object-quality hypothesis; fitted scale is not recovered physical scale.

The corrected runner now validates raw FSSR and native cleaner payloads while allowing finite zero-area triangles until the deterministic final sanitation stage; its final `mesh.ply` remains subject to strict nonzero-area validation. A fresh full-photo rerun has not yet exercised this corrected runner. A separate fresh CMake-binary tree smoke did run after fixing a macOS PNG header/library mismatch: 9/10 cameras and 69,676 mesh faces in 54.44 s, versus 69,897 faces from the GNU build. This indicates numerical/build variation; neither is an object-quality benchmark. The CMake build now includes a PNG ABI test, and the scoped CI runs it, but Linux and Windows runtime paths remain unverified.
