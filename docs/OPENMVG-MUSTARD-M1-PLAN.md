# OpenMVG v2.1 mustard CPU SfM preflight (no build or photo run)

## First approved build attempt, 2026-09-27

One explicitly approved `--build` attempt cloned and sealed the exact source
and all three submodules, then stopped at CMake configuration; **no target
compiled, no photo staged, and no SfM ran**. Installed CMake 4.1.1 rejects
the vendored `src/dependencies/osi_clp/CMakeLists.txt:6` declaration
`CMAKE_MINIMUM_REQUIRED(VERSION 2.6)` because compatibility below 3.5 was
removed. The earlier “Osi not found” message causes the vendored Osi/CLP
fallback to be configured; it is not itself the fatal diagnostic. CMake
suggests `-DCMAKE_POLICY_VERSION_MINIMUM=3.5` as a possible override; the
separately approved test and its subsequent blocker are recorded below.
Older-CMake/toolchain or source patch options need a separate reviewed attempt. Do not rerun
`--build` on the now nonfresh root: its fresh-root guard correctly refuses.

The external evaluation tree is 101 MiB after the stop. The final device
checks showed 14,981,548 KiB free externally and 23,261,852 KiB internally,
above both 11 GiB operating guards; the source checkout remains clean.
The original `build-manifest.json` SHA-256 (now preserved as
`build-manifest-attempt1.json`) was
`e328f483306fe0979291a8c74e0c3016a547283d37b725fe63e20958a97327e8`;
`logs/04-configure.log` SHA-256 is
`c12becb2b57a76590f239730bc5960b19befb862cb310df65f50dc242d6696f6`.
These artifacts live under
`/Volumes/backups/code/crispmesh-data/openmvg-mustard-v21-001`; do not mutate
them while planning the next controlled attempt.

A separate `--resume-preflight` is read-only and succeeds only when the first
manifest and configure log still match those exact hashes, source HEAD,
gitlinks, clean status, license-file hashes and first-stage statuses match,
no photo directory exists, no target build log exists, and both disk guards
still pass. On 2026-09-27 it reported 97,482,152 managed bytes,
15,341,080,576 external free bytes versus 13,861,161,560 required, and
23,817,494,528 internal free bytes. The explicitly approved
`--resume-configure-build` snapshotted the first manifest byte-for-byte
as `build-manifest-attempt1.json`, then appended a distinct bounded stage
record/log (`06-resume-configure.log`); the planned `07-resume-build.log`
was never created. It used only the
additional `-DCMAKE_POLICY_VERSION_MINIMUM=3.5` configure flag. It retains
LiGT OFF, all target/resource/time/cache guards, the clean source and all
three exact submodule revisions. The reviewed supervisor code SHA-256 for
this resume was `436b15eee93778cce27007681a5cbe0004f58023a3436dc5165d559612994c52`;
11 synthetic guardrail tests pass.

The one separately approved resume ran and stopped in `06-resume-configure.log`
before compilation. The CMake policy override cleared the first blocker,
but vendored Ceres 1.13 then rejected an empty detected Eigen version (`..`).
Its `cmake/FindEigen.cmake` reads `EIGEN_WORLD_VERSION` and related macros from
`Eigen/src/Core/util/Macros.h`; the selected Homebrew
`/opt/homebrew/include/eigen3` places its version macros in `Eigen/Version`
instead. That installation reports Eigen 3.5.0 with patch level 1. A separate
local Eigen 3.4.0 header tree has the legacy macros, but no hint/cache change,
dependency install or further retry was made. A future attempt must review
the exact Ceres/Eigen ABI and CMake selection, not silently patch the source.
The first manifest snapshot remains byte-identical (SHA-256
`e328f483306fe0979291a8c74e0c3016a547283d37b725fe63e20958a97327e8`);
the appended manifest SHA-256 is
`a762484396402bc37c69a4765a7c7497ba0e7b50d7680d0fb75faccd8167d80a`,
and `06-resume-configure.log` SHA-256 is
`1486702aa41731a4a45d5884f3eeff7c2238053cb18a0d9759e7e370205c388a`.
No `07-resume-build.log`, staged photo or SfM output exists. The tree is still
101 MiB, with 14,980,712 KiB free externally and 23,242,376 KiB internally.

## Eigen 3.4 attempt 3 and build-only continuation

The source-unmodified version-pinned retry adds exactly two CMake hints to
the prior policy-override configure command:

```text
-DEigen3_DIR:PATH=.local-tools/bundle-quality/eigen-install/share/eigen3/cmake
-DEIGEN_DIR:PATH=.local-tools/bundle-quality/eigen-install/include/eigen3
```

OpenMVG's top-level `find_package(Eigen3)` uses the first cached config path;
its legacy `FindEigen.cmake` uses the second cached include path. Vendored
Ceres's `FindEigen.cmake` prefers the exported Eigen3 config and then reads
version macros from `Eigen/src/Core/util/Macros.h`. The local Eigen3 config
reports 3.4.0, and installed `Macros.h` defines version 3.4.0 and matches
the local source byte-for-byte. The local archive SHA-256 is
`8586084f71f9bde545ee7fa6d00288b264a2b7ac3607b974e54d13e7162c1c72`;
installed `Macros.h` SHA-256 is
`8d73259b4ba482e6dbd11b31d8887fd350dfb0836700d32793446bba6ab1ae7a`.
The exported `Eigen3Config.cmake` SHA-256 is
`c74a58d3437cd9b1d5503d627962a323f8b9a8f884bfae65f8206e63560a45a4`.
[Official Eigen 3.4.0 release](https://gitlab.com/libeigen/eigen/-/releases/3.4.0),
[local dependency provenance](BUNDLE-QUALITY.md).

`--resume-eigen-preflight` is read-only and requires the clean sealed OpenMVG
source; unchanged first-attempt snapshot, second-attempt manifest and 04/06
configure logs; no 05/07 build logs, 08/09 logs, or photo/SfM outputs; the
exact audited attempt-2 CMake cache SHA-256
`a3531b4fb9a3bd17018d308db41b63b45bdbfb2744f9da485353715790e23bf0`;
and the local Eigen 3.4 source/archive/header/config/license hashes. The
separate explicit `--resume-eigen-configure-build` snapshots the second
manifest as `build-manifest-attempt2.json`, then uses new bounded 08/09 logs
and all existing 1.5 GiB build, 2 GiB total, 11 GiB device, 4 GiB RSS,
16 MiB per-log, two-thread and timeout guards. Before compilation it requires
the Ceres configure log to report Eigen 3.4.0, CMake cache entries to point
to the local install, and generated Ninja compile rules to use that include
path with no Homebrew Eigen path. It does not install dependencies or stage
photos. On 2026-09-27 the read-only preflight passed: 97,876,707 managed
bytes, 15,340,638,208 external free bytes versus 13,860,767,005 required,
and 23,799,353,344 internal free bytes. The supervisor SHA-256 for review is
`14c04261daa54f8504ec6da9719d38f6dd391dbddc648cd1c18e36586d3941f7`;
15 synthetic tests passed at that point.

The one approved attempt-3 configure completed successfully: vendored Ceres
reported Eigen 3.4.0 from the pinned local include tree, the cache pinned
both `Eigen3_DIR` and `EIGEN_DIR` locally, LiGT remained OFF, and generated
`build.ninja` uses the local include path without a Homebrew Eigen path.
Compilation did **not** start. The supervisor had additionally required
`EIGEN_INCLUDE_DIR` to appear in `CMakeCache.txt`, but Ceres held it as a
non-cache CMake variable. That unsupported cache-entry expectation stopped
the workflow after configure; it is a guard mismatch, not a CMake failure.
The second-attempt manifest snapshot SHA-256 is
`a762484396402bc37c69a4765a7c7497ba0e7b50d7680d0fb75faccd8167d80a`;
the configured manifest SHA-256 is
`924dcf91e1878dec8f0f2c78300fb89eb05d7845e9decfedb9a8b6b74ce90ab7`;
`08-eigen-configure.log` SHA-256 is
`1b313e4e15111c8ee514df9a78bdbf29665de6b9deae06bd952aaa1c490b8023`;
the generated CMake cache SHA-256 is
`6560dd0a3ed4530150fdd36b0c98b0dd66dad4fdeedf511ca33fff1d72d561d0`.

The proposed **build-only** continuation has a separate
`--resume-eigen-build-preflight` and explicit `--resume-eigen-build-only`.
It seals the original two manifest snapshots, current configured manifest,
04/06/08 logs, generated cache, clean source/gitlinks/license inventory and
local Eigen 3.4 files, and requires no photo/SfM outputs or prior target
build log. The corrected post-configure gate requires cached `Eigen3_DIR`
and `EIGEN_DIR` to point locally; if `EIGEN_INCLUDE_DIR` is cached it must
also point locally, but its absence is valid. Independently, the 08 log must
show Ceres finding Eigen 3.4.0 at that path and generated Ninja rules must
contain that path and no Homebrew Eigen path. It snapshots the configured
manifest before running only the four predeclared target builds into the new
bounded `09-eigen-build.log`; it does not rerun configure. The existing
resource guards remain unchanged. The read-only build preflight passed with
99,691,832 managed bytes, 14,664,273,920 external free bytes versus
13,858,951,880 required, and 23,841,271,808 internal free bytes. The
supervisor SHA-256 for build-only review is
`f5e0aac453d4976df44320368d30261c7fc3fa6f42fed4347d408440a187e165`;
16 synthetic tests pass. **Do not run the build-only continuation before
explicit approval.**

The one approved build-only continuation was deliberately stopped at Ninja
step 303/388, after 175.219 seconds, because its target closure compiled
`openMVG/multiview/LiGT/LiGT_algorithm.cpp` and
`LiGT_algorithm_converter.cpp` despite `OpenMVG_USE_LIGT=OFF`. The first
source file expressly names CC BY-SA 4.0, and the top-level CMake warns of a
LiGT patent condition. `openMVG/multiview/CMakeLists.txt` includes
`./LiGT/*.cpp` in `multiview_files_cpp`, while its OFF branch tries to remove
`LiGT_*.cpp`; the generated Ninja graph confirms those files remained in
`openMVG_multiview`. This is a source-level build-closure/licensing surprise,
not an algorithm result. The agent sent SIGTERM only to the build process
group; Ninja's “interrupted by user” is the consequence of that deliberate
stop. The supervisor recorded `status=stopped`, `returncode=-15`, not a disk,
RSS, timeout or compiler failure. No four-target build completed and no photo
or SfM stage ran. A source/CMake fix and another license inventory would need
separate review before any retry; evaluation-only permission does not create
a shipping-license conclusion.

The configured manifest snapshot SHA-256 is
`924dcf91e1878dec8f0f2c78300fb89eb05d7845e9decfedb9a8b6b74ce90ab7`;
the stopped build manifest SHA-256 is
`1b15ae1fc40719f8116a68ea465d2c57062d96f8c657aa54a9bb7f736aaf3806`;
`09-eigen-build.log` SHA-256 is
`253c9a535e99de4b5163e31f8dc433a85321d0fd1586933104a339acbabbd7f6`.
The managed tree was about 147 MiB at stop; final external free space was
14,276,528 KiB and internal free space 23,276,764 KiB. The supervisor,
CMake and Ninja processes were confirmed absent afterward.

This pin addresses a configuration compatibility error, not a shipping
license determination. Eigen 3.4 is primarily MPL-2.0 but includes optional
LGPL files; its `COPYING.README` prescribes `EIGEN_MPL2_ONLY` as a compile-time
guard. Current failed cache settings include `EIGENSPARSE=ON` and `LAPACK=ON`;
vendored Ceres emits an LGPL warning with Eigen sparse enabled. Those linked
targets and App Store constraints need separate review before any shipping
claim. [Eigen 3.4 license notice](https://gitlab.com/libeigen/eigen/-/blob/3.4.0/COPYING.README).

## Decision and scope

**Conditional go for one isolated M1 evaluation build and 48-TRAIN-view CPU
SfM comparison; no-go for shipping now.** The immutable candidate is upstream
OpenMVG `v2.1`, commit `01193a245ee3c36458e650b1cf4402caad8983ef`
(tag checked by read-only `git ls-remote`, release published 2023-12-28).
The pinned source and submodules are now cloned and sealed in an external
evaluation tree; configuration succeeded with local Eigen 3.4, but no target
has compiled.
The candidate's core is MPL-2.0, which fits the project's prospective shipped
third-party policy only after exact binary/dependency, notices, MPL source
archive and cross-platform packaging review. The current project code itself
is AGPL-3.0-only; this plan does not change that. [OpenMVG license](https://github.com/openMVG/openMVG/blob/01193a245ee3c36458e650b1cf4402caad8983ef/LICENSE),
[release](https://github.com/openMVG/openMVG/releases/tag/v2.1),
[local dependency policy](DEPENDENCIES.md).

The present external backup device has 15,086,732 KiB free, about 14.39 GiB,
or about 4.39 GiB above the 10 GiB floor; the internal workspace device has
23,424,000 KiB free. `/Volumes/backups/ai` and
`/Volumes/backups/code/crispmesh-data` share that backup device. A separate
AliceVision Meshroom DMG is 1,247,000,900 bytes before extraction; do not
overlap its provisioning with this build. [AliceVision local audit](ALICEVISION-MAC-ORACLE.md).

## Source and dependency blockers

At this v2.1 pin, CMake's `OpenMVG_USE_LIGT` defaults **ON**, and the source
states that LiGT has a patent concern and a CC BY-SA 4.0 condition. Explicitly
set `-DOpenMVG_USE_LIGT=OFF` and verify it is absent from the built target
closure. The top level also unconditionally adds `nonFree/sift`; the normal
`openMVG_main_ComputeFeatures` target links `vlsift` even if the chosen runtime
method is `SIFT_ANATOMY` or `AKAZE_FLOAT`. The wrapper
`nonFree/sift/SIFT_describer.hpp` states MPL-2.0, while its vendored VLFeat
`vl/sift.c` names BSD terms; the upstream VLFeat `COPYING` is BSD-style.
Thus `nonFree` is **not a demonstrated GPL component**, but its historical
patent/research labeling, vendored notice provenance and unavoidable target
link are a shipping gate. A later shipping configuration should remove the
unused `vlsift` dependency from the feature CLI and audit the resulting
binary; this evaluation plan does not assume that patch works.
[Pinned top-level CMake](https://github.com/openMVG/openMVG/blob/01193a245ee3c36458e650b1cf4402caad8983ef/src/CMakeLists.txt),
[feature target](https://github.com/openMVG/openMVG/blob/01193a245ee3c36458e650b1cf4402caad8983ef/src/software/SfM/CMakeLists.txt),
[wrapper](https://github.com/openMVG/openMVG/blob/01193a245ee3c36458e650b1cf4402caad8983ef/src/nonFree/sift/SIFT_describer.hpp),
[VLFeat file](https://github.com/openMVG/openMVG/blob/01193a245ee3c36458e650b1cf4402caad8983ef/src/nonFree/sift/vl/sift.c),
[VLFeat terms](https://github.com/vlfeat/vlfeat/blob/master/COPYING).

Minimal M1 build preflight: pin submodule commits from the v2.1 tree before
fetch; configure Release, arm64, `OpenMVG_BUILD_TESTS=OFF`,
`OpenMVG_BUILD_DOC=OFF`, `OpenMVG_BUILD_EXAMPLES=OFF`,
`OpenMVG_BUILD_GUI_SOFTWARES=OFF`, `OpenMVG_BUILD_OPENGL_EXAMPLES=OFF`,
`OpenMVG_USE_OPENCV=OFF`, `OpenMVG_USE_LIGT=OFF`, and
`OpenMVG_USE_OPENMP=OFF` unless the exact M1 toolchain proves OpenMP support.
Build only `openMVG_main_SfMInit_ImageListing`,
`openMVG_main_ComputeFeatures`, `openMVG_main_ComputeMatches`,
`openMVG_main_SfM`, and later, if SfM is accepted,
`openMVG_main_ConvertSfM_DataFormat`/a dense-export target. Do **not** run
unqualified `install`/`all`: the CMake tree defines GUI, examples and
nonFree targets separately. The local workspace already has arm64-oriented
Eigen and Ceres installations under `.local-tools/bundle-quality` (9.4 MiB
and 19 MiB), but their version/ABI and target graph against OpenMVG v2.1
are unverified. CMake can fall back to bundled Eigen/Ceres and other image,
matching and optimization libraries, so the target dependency closure and
actual linked files must be captured before release. Upstream documents
Mac, Linux and Windows builds, but no target-platform package or App Store
compatibility is established here. [Pinned build CMake](https://github.com/openMVG/openMVG/blob/01193a245ee3c36458e650b1cf4402caad8983ef/src/CMakeLists.txt),
[third-party CMake](https://github.com/openMVG/openMVG/blob/01193a245ee3c36458e650b1cf4402caad8983ef/src/third_party/CMakeLists.txt),
[build instructions](https://github.com/openMVG/openMVG/blob/01193a245ee3c36458e650b1cf4402caad8983ef/BUILD.md).

The sealed v2.1 gitlinks are `src/dependencies/cereal`
`ac168fe78ac499be0a745bf4a3253a9660572f8d`,
`src/dependencies/glfw` `eab31f228fd1872753c55fb438e3d88e51c0e9b2`,
and `src/dependencies/osi_clp`
`a25a980c1af50cbd8962fe9d21035afa03653270`. The supervisor verifies
these against both the parent git tree and actual submodule checkouts, plus
their exact `.gitmodules` URLs, before configuration. The source/license
inventory hashes `LICENSE`, `COPYRIGHT.md`, top-level and SfM CMake files,
the SIFT wrapper and VLFeat C file, `.gitmodules`, and discovered submodule
license files. It stops if a CMake file declares an implicit
`ExternalProject`, `FetchContent` or `file(DOWNLOAD)` operation. This is a
source audit, **not** a transitive binary-license clearance; the upstream
notice lists optional LGPL CXSparse, which must be excluded and verified in
the eventual linked target closure. [Pinned gitmodules](https://github.com/openMVG/openMVG/blob/01193a245ee3c36458e650b1cf4402caad8983ef/.gitmodules),
[pinned third-party notice](https://github.com/openMVG/openMVG/blob/01193a245ee3c36458e650b1cf4402caad8983ef/COPYRIGHT.md).

GitHub reports about 30 MB of repository data, which excludes an exact
submodule checkout/build estimate. Allow **at most 2 GiB total new external
bytes**, including pinned source/submodules, build tree, copied/staged images,
matches, sparse output, logs and temporary files; split 1.5 GiB build and
0.5 GiB experiment. This is a stop cap, not a claim that compilation fits.
Use at most two compile and SfM threads, 4 GiB resident memory, 90 minutes
build and 20 minutes photo pipeline. Preflight both devices above 11 GiB and
the backup device above 10 GiB + the remaining cap + 1 GiB guard; monitor
both devices and the external output tree throughout. If the AliceVision DMG
must be downloaded/extracted, reserve it separately and **do not start** the
OpenMVG build under the present shared-device headroom. No caches in `/tmp`
or the home directory.

The implementation is [the evaluation supervisor](../scripts/classical_backend/openmvg_m1_supervisor.py).
Its default invocation, `python3 scripts/classical_backend/openmvg_m1_supervisor.py`,
is **read-only**: it hashes the already sealed TRAIN manifest, 48 JPEGs and
48 masks, reports device free space, and reports the pending build/source
inventory. No clone or build is implied by preflight. A separate, future
operator-approved `--build` would create only the fresh
`/Volumes/backups/code/crispmesh-data/openmvg-mustard-v21-001` tree (source,
build, logs, scratch) and build exactly the four listed CLI targets with two
compile threads. A later `--stage` copies only the hashed TRAIN JPEGs and
renamed masks into `images48`; it does not run an SfM command. The managed
tree, including git metadata, build and staging, is capped at 2 GiB; the
build allocation is 1.5 GiB and the photo allocation 0.5 GiB. The guarded
subprocess loop checks both device floors at 11 GiB, the managed tree size,
and descendant-process RSS at 4 GiB; it terminates the process group on a
breach or timeout. Clone, checkout, submodule update, configure and build
each have a separate 16 MiB maximum stdout/stderr log under `logs`; overflow
stops the process group. `build-manifest.json` is written before each command
and updated with its exact argument vector, log path, elapsed time, exit
status or stop reason, plus the sealed source/license inventory when available.
All `TMPDIR`/`TMP`/`TEMP` and XDG cache paths for the build
are under the external tree. It never invokes Homebrew or a package manager.
No `/tmp` workspace, image/weights download, all-target build, photo run, or
dense export is part of this approval stage. The synthetic guardrail tests are
in [the supervisor test](../scripts/classical_backend/test_openmvg_m1_supervisor.py).

## Frozen TRAIN-only camera comparison

Use only the sealed 48 photos in
`/Volumes/backups/code/crispmesh-data/mustard-sfm-train-001/images` and its
reviewed coarse feature masks. The staged JPEGs total 47 MiB; masks total
192 KiB. The `train-names.txt` SHA-256 is
`a81d1647109db27991c39c5fe1a9ae308c04081f8ada49d8b0f5ab1c15245544`;
the stage report SHA-256 is
`bbcba624f6a51f82e0ebdebf6e7e4cda10804bc8df34cfd0d5b3242983a225d0`.
Create a fresh, exact 48-photo image directory on the external device and
corresponding masks, with source hashes checked before and after. Rename
`NP3_006.jpg.png` to OpenMVG's required `NP3_006_mask.png` convention
(analogously for all 48); confirm binary values, image dimensions and mask
support after staging. OpenMVG's pinned feature source constructs the name
from the image basename plus `_mask.png`, prefers per-image masks, and
ignores mask images during image listing. Do not include any of the 12
held-out photos, scanner/depth/reference mesh or supplied Berkeley poses.
[Pinned feature mask lookup](https://github.com/openMVG/openMVG/blob/01193a245ee3c36458e650b1cf4402caad8983ef/src/software/SfM/main_ComputeFeatures.cpp),
[image-listing source](https://github.com/openMVG/openMVG/blob/01193a245ee3c36458e650b1cf4402caad8983ef/src/software/SfM/main_SfMInit_ImageListing.cpp).

The minimum source-level command contract, to verify against each built
binary's `--help` before execution, is:

```text
openMVG_main_SfMInit_ImageListing -i IMAGES48 -o MATCHES -f 1536 -c 2 -g 1
openMVG_main_ComputeFeatures -i MATCHES/sfm_data.json -o MATCHES -m SIFT_ANATOMY -p NORMAL
openMVG_main_ComputeMatches -i MATCHES/sfm_data.json -o MATCHES -g e -r 0.8
openMVG_main_SfM -i MATCHES/sfm_data.json -m MATCHES -o SPARSE -s INCREMENTAL -f NONE
```

`-f 1536` is the same unmeasured 1.2×1280 fixed initial focal assumption as
the PyCOLMAP baseline, `-c 2` is one-radial pinhole, and `-g 1` permits a
shared intrinsic group. SfM `-f NONE` holds it fixed, with no supplied seed
pair or prior pose. This is a new extractor/matcher/mapper **pipeline**
comparison, not a mapper-only test on identical features. The default OpenMVG
matching ratio is 0.8, but pin it explicitly; use essential geometry since
the camera assumption is fixed. Capture actual image-listing focal/principal
point, image IDs, feature and geometric-match counts, SfM seed, software
hashes, output sizes, wall time and peak RSS. [Image listing](https://openmvg.readthedocs.io/en/latest/software/SfM/SfMInit_ImageListing/),
[features and masks](https://openmvg.readthedocs.io/en/latest/software/SfM/ComputeFeatures/),
[matching](https://openmvg.readthedocs.io/en/latest/software/SfM/ComputeMatches/),
[pinned SfM options](https://github.com/openMVG/openMVG/blob/01193a245ee3c36458e650b1cf4402caad8983ef/src/software/SfM/main_SfM.cpp).

Reusing the sealed PyCOLMAP SQLite database is **not a direct option**:
OpenMVG expects `sfm_data.json` plus its own per-image regions/descriptors
and geometric match files, with different image identifiers and serialization.
An adapter would need to preserve exact feature indices, descriptor
representation, pair orientation and verification semantics; that is a
separate ablation. Fresh OpenMVG extraction is the bounded comparison here.
[OpenMVG data structures](https://github.com/openMVG/openMVG/wiki/OpenMVG-data-structures),
[COLMAP database format](https://colmap.github.io/format.html#database-format).

## Predeclared acceptance and downstream gate

Require all 48 TRAIN cameras to register; 24/48 partial arcs fail. On the
48-camera model compute the existing train-only trajectory statistics:
camera centers `-Rᵀt`, center PCA plane and median planar radius, 47 adjacent
steps plus closure, and all 24 cyclic-opposing pairs. Reject a folded-orbit
candidate if the median opposing center distance/radius is `<= 1.0`, **or**
any opposing pair is `<= 0.5` and has a full-orientation difference `<= 15°`,
**or** adjacent-step p95 exceeds `1.0` radius or adjacent full-rotation p95
exceeds `45°`. These coarse, predeclared gates distinguish the existing
exhaustive fold (median opposing 0.179; adjacent p95 2.597 radius/81.69°)
without pretending acquisition labels are measured angles. Also require the
two frozen opposing TRAIN pairs `030/198` and `036/222` to pass the prior
E/cheirality/parallax and competing-homography gates in as-stored,
canonical and reverse row order with the same three seeds; report
preselected leave-one-out sensitivity and withhold a physical-pose claim if
either pair is order/removal unstable. Freeze all of this before inspecting
an OpenMVG camera. No reference geometry, held-out photo or depth may tune
these settings. [Existing trajectory diagnostic](MUSTARD-POSE-DIAGNOSTIC.md),
[two-view gate](MUSTARD-TWO-VIEW-POSE-PLAN.md),
[robustness finding](MUSTARD-USAC-ROBUSTNESS-RESULT.md).

Only after these camera and correspondence gates pass should a dense export
be planned. OpenMVG documents an `openMVG_main_openMVG2openMVS` bridge for
OpenMVS and an MVE export path; either requires its own image/pose-coordinate
and mask-handoff checks. OpenMVS is AGPL evaluation-only under the current
product boundary, and existing OpenMVS dense comparisons cannot certify this
new camera model. [OpenMVG to OpenMVS](https://openmvg.readthedocs.io/en/latest/software/MVS/OpenMVS/),
[OpenMVG to MVE](https://openmvg.readthedocs.io/en/latest/software/MVS/MVE/),
[local dependency policy](DEPENDENCIES.md).
