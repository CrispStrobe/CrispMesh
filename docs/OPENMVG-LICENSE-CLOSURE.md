# OpenMVG v2.1 four-CLI license-closure audit (evaluation only)

## Evidence boundary

Pinned upstream source is OpenMVG `01193a245ee3c36458e650b1cf4402caad8983ef`
with `osi_clp` gitlink `a25a980c1af50cbd8962fe9d21035afa03653270`.
The generated Ninja graph for `openMVG_main_SfMInit_ImageListing`,
`openMVG_main_ComputeFeatures`, `openMVG_main_ComputeMatches` and
`openMVG_main_SfM` enumerates **363 distinct direct C/C++ source files**.
A source-text screen for GPL/LGPL, CC-SA, noncommercial and patent terms
matched six of those: two LiGT CC BY-SA files, one Bison-generated CoinUtils
file with a GPL skeleton notice and exception, and three non-license keyword
hits (`USE_PATENTED_LIGT` references in two files and `flugpl` in one).
This screen does not prove the license of every included header or final linked
object; the stopped build produced no four target executables for an `otool`
or binary-symbol audit. The exact external build manifest/log hashes and
manual SIGTERM stop are recorded in [the M1 plan](OPENMVG-MUSTARD-M1-PLAN.md).

## Findings that block a shipping claim

| Evidence in pinned target closure | Finding / unresolved condition |
| --- | --- |
| `openMVG/multiview/LiGT/LiGT_algorithm.cpp` and `LiGT_algorithm_converter.cpp` | Both are compiled into `openMVG_multiview` with `OpenMVG_USE_LIGT=OFF`, contrary to the intended CMake gate. They declare CC BY-SA 4.0; top-level CMake separately warns of LiGT patent conditions. This is why the build was stopped. [Pinned CMake](https://github.com/openMVG/openMVG/blob/01193a245ee3c36458e650b1cf4402caad8983ef/src/openMVG/multiview/CMakeLists.txt), [LiGT source](https://github.com/openMVG/openMVG/blob/01193a245ee3c36458e650b1cf4402caad8983ef/src/openMVG/multiview/LiGT/LiGT_algorithm.cpp). |
| `third_party/cmdLine/cmdLine.h` | **All four CLI mains directly include this header.** Its own notice says LGPL-3.0-or-later, while upstream `COPYRIGHT.md` describes `cmdLine` as MPL-2.0. That conflict is unresolved; do not rely on the summary notice to clear shipping. Header SHA-256 `aa3f87d1c33822745eff5d1c993905b3791308e9823f34f044796459e9cab5ba`. [Pinned header](https://github.com/openMVG/openMVG/blob/01193a245ee3c36458e650b1cf4402caad8983ef/src/third_party/cmdLine/cmdLine.h), [upstream notice](https://github.com/openMVG/openMVG/blob/01193a245ee3c36458e650b1cf4402caad8983ef/COPYRIGHT.md). |
| `CoinUtils/src/CoinModelUseful2.cpp`, `lib_CoinUtils.a`, Clp/Osi | Ninja compiles the parser source into `lib_CoinUtils.a`, and all four CLI link recipes name that archive along with Clp/Osi. The file includes a GPL-2.0-or-later Bison skeleton notice **and a special exception for Bison output**; this is not an automatic GPL verdict. CoinUtils/Clp/Osi bundled LICENSE files are EPL-1.0, another redistribution/compatibility review. Parser SHA-256 `c00f7cc23e4e9697c5eae094029c50c21ce8b6e0f7eb532dc7a92e0f10aa66c0`; EPL file SHA-256 `0076749b626931ea5aaee25ddd5019fbfd96da78243cc0d5bd24fe246500981f`. [Pinned parser](https://github.com/openMVG-thirdparty/osi_clp/blob/a25a980c1af50cbd8962fe9d21035afa03653270/CoinUtils/src/CoinModelUseful2.cpp), [EPL notice](https://github.com/openMVG-thirdparty/osi_clp/blob/a25a980c1af50cbd8962fe9d21035afa03653270/CoinUtils/LICENSE). |
| Vendored Ceres 1.13, local Eigen 3.4 | `EIGENSPARSE=ON` makes vendored Ceres print an LGPL warning. Generated Ceres compile rules also define `EIGEN_MPL2_ONLY`, and Eigen 3.4's own notice says that macro rejects LGPL headers. These are conflicting/dated indicators, not a final linked-license determination. `CXSPARSE=OFF`, `SUITESPARSE=OFF`, but `LAPACK=ON` links Apple Accelerate; App Store/symbol review remains. [Pinned Ceres CMake](https://github.com/openMVG/openMVG/blob/01193a245ee3c36458e650b1cf4402caad8983ef/src/third_party/ceres-solver/CMakeLists.txt), [Eigen 3.4 notice](https://gitlab.com/libeigen/eigen/-/blob/3.4.0/COPYING.README). |
| `libvlsift.a` and system libraries | The feature CLI link recipe names vendored `vlsift` even when runtime `SIFT_ANATOMY` is selected. Its VLFeat source says BSD, but `nonFree` provenance/patent review remains. The four link recipes also name Homebrew JPEG/PNG/TIFF dylibs and Apple Accelerate; exact shipped binaries, notices and platform packaging have not been audited. [Pinned feature CMake](https://github.com/openMVG/openMVG/blob/01193a245ee3c36458e650b1cf4402caad8983ef/src/software/SfM/CMakeLists.txt), [VLFeat file](https://github.com/openMVG/openMVG/blob/01193a245ee3c36458e650b1cf4402caad8983ef/src/nonFree/sift/vl/sift.c). |

This is an **evaluation-oracle closure**, not an App Store or commercial
clearance. In particular, the LiGT-only patch below does not resolve the
`cmdLine` header, EPL/Clp, Ceres/Eigen or linked-library questions.

## Minimal LiGT-OFF source patch proposal

The in-repo [patch](../scripts/classical_backend/openmvg_v21_ligt_off.patch)
changes only `src/openMVG/multiview/CMakeLists.txt`: it uses the same
`./LiGT/*.cpp` and `./LiGT/*.hpp` paths as the initial file globs for the
OFF-branch removal, and removes headers from `multiview_files_header` rather
than from `multiview_files_cpp`. Patch SHA-256:
`472b4a925cc5e92fc41c733777d55b4cee2d8944764441537f69689968510f2c`.
It passes `git apply --check` against the clean pinned external checkout;
the patch has **not** been applied there. The exact original CMake fixture in
[the fixture](../scripts/classical_backend/fixtures/openmvg_v21_multiview_CMakeLists.txt)
matches upstream bytes (SHA-256
`139ed9e2793e907c3fa74ea805c5b418336f51812ab7c56036aeb14e866233a3`).
The [fixture test](../scripts/classical_backend/test_openmvg_ligt_off_patch.py)
applies the patch only in a temporary directory and checks the OFF exclusion
of synthetic LiGT source/header paths. This is an **oracle-build fix only**.

If approved later, copy the sealed source into a fresh local external
fork-worktree under
`/Volumes/backups/code/crispmesh-data/openmvg-v21-ligt-off-oracle-001`,
verify base HEAD/gitlinks and patch hash, apply it there, and record the
new commit/tree hash. Do not alter the preserved source or create a GitHub
fork/PR. Keep a **1 GiB total new-tree cap**, both devices at least 11 GiB
free, at most two compile threads and 4 GiB process-tree RSS, 16 MiB/stage
logs, external scratch only, and the same four target/time limits. A fresh
read-only disk reservation must pass before copying. Configure with the
already pinned Eigen 3.4 and `OpenMVG_USE_LIGT=OFF`, additionally generating
`compile_commands.json`; before any compile, assert no `LiGT/*.cpp` appears
there, in the four-target `ninja -t commands` closure, or in the
`openMVG_multiview` Ninja archive inputs, and assert `USE_PATENTED_LIGT` is
absent from compile definitions. After a build, inspect archive members and
linked binaries. Stop on any mismatch or new license finding; do not use
photos until the bounded binary and camera-stage gates are separately
approved.
