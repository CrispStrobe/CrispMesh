# OpenMVG GeometricFilter fifth-target build result

The separately approved, evaluation-only `openMVG_main_GeometricFilter`
build completed once on the sealed OpenMVG v2.1 LiGT-off fork. The saved log
shows exactly two steps—compile `main_GeometricFilter.cpp`, then link the
executable—with return code 0 in 7.184 seconds. It contains one linker
warning about duplicate library arguments, not a failed link. No photo,
matching, or SfM stage ran.

Independent read-only verification on 2026-09-27:

| Artifact/check | Result |
| --- | --- |
| [Fifth build manifest](/Volumes/backups/code/crispmesh-data/openmvg-v21-ligt-off-oracle-001/geometric-filter-build-manifest.json) | SHA-256 `4b3dc5ac03ea2232067bebabc0b4d41dc5bfb91076bf0e31211162f32842774b`; status `built_fifth_oracle_only`. |
| [Fifth license receipt](/Volumes/backups/code/crispmesh-data/openmvg-v21-ligt-off-oracle-001/geometric-filter-license-receipt.json) | SHA-256 `49db5dca401c827cac53a5b1337256effd198ed36de23de283b2b32548e53dc2`; binary hash and live `otool -L` output match it. |
| [Build log](/Volumes/backups/code/crispmesh-data/openmvg-v21-ligt-off-oracle-001/logs/06-geometric-filter-build.log) | SHA-256 `b1ca30c60c465046f9a31dda6f8f96b4df32c93a915beb29252c70547ab0d299`; two steps, no retry. |
| [GeometricFilter executable](/Volumes/backups/code/crispmesh-data/openmvg-v21-ligt-off-oracle-001/build/Darwin-arm64-Release/openMVG_main_GeometricFilter) | SHA-256 `49a5ca029356f3f8bb3eb68eaae58d98f77204b2b4cdf85f52aa0448f0ab1b77`. |
| Source and graph | Original source/Eigen and patched-fork seals pass; Git status shows only the reviewed `multiview/CMakeLists.txt` modification. Generated and frozen Ninja manifests both retain SHA-256 `4dc33b80221bdf9f9933d0fba33e207b12bfaa0009d0e7560d70263a4a8d808b`; fifth main retains SHA-256 `087875a3122f64c328c365e7e73db752830ddb709ed7ecbda1a075c241ff40fa`. All four previous binary/archive seals pass. |
| Resource/photo gate | Fork tree 186,975,714 bytes (<1 GiB); external free 14,444,421,120 bytes and internal free 22,946,619,392 bytes (both >11 GiB). No `images48`, `matches`, or `sparse` output exists. |

The generated fifth-target command closure introduced only its MPL-declared
CLI main and link step, with no new dependency library. It directly includes
the same LGPLv3+ `cmdLine.h` already flagged in the four-target audit; the
receipt confirms no new dynamic dependency relative to those binaries.
This remains an **evaluation oracle, not a shipping clearance**. The
LGPL-versus-upstream-MPL `cmdLine` conflict, EPL Clp/Osi/CoinUtils and Bison
exception, Ceres/Eigen sparse-license question, VLFeat/nonFree provenance,
and Homebrew JPEG/PNG/TIFF dynamic-library packaging still require separate
resolution. See the [license-closure audit](OPENMVG-LICENSE-CLOSURE.md) and
[fifth-target plan](OPENMVG-GEOMETRIC-FILTER-FIFTH-TARGET-PLAN.md).
