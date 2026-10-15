# OpenMVG LiGT-off fork: frozen four-target oracle build gate

This is an **evaluation-only** build plan, not a commercial/App Store license
clearance and not a photo/SfM approval. The configured fork is fixed at
`/Volumes/backups/code/crispmesh-data/openmvg-v21-ligt-off-oracle-001`.
The preserved original source under `openmvg-mustard-v21-001` is never built or
edited by this continuation. No clone, package install, network fetch, image
stage, or SfM run is part of this command.

## Frozen input and output contract

- OpenMVG v2.1 HEAD `01193a245ee3c36458e650b1cf4402caad8983ef`;
  only local source difference is the reviewed LiGT-OFF CMake patch
  SHA-256 `472b4a925cc5e92fc41c733777d55b4cee2d8944764441537f69689968510f2c`.
  Patched `multiview/CMakeLists.txt` SHA-256
  `da54e79ecfee54b1eccb429095de5d7a4172a125b4bf67c8a2709a6eb846a1ae`.
- Configured-fork manifest SHA-256
  `5dc2a067b492216166338d8035d6db0dd12149672965a09d7c89635f1878d00a`;
  CMake cache `e40359026ce9a4cff60e8e5dfa6781b042ff62d79ac6867948ad8ef0f9814769`;
  configure log `0493235a98712ac837a1690678c7dfe780ae5fc66125db0f3b72e81a850cf18c`.
  The two patch logs are empty (SHA-256 `e3b0c442...`); four-target command
  closure log SHA-256
  `847b30da11a793671d480b22901ffc871b793728d178e07a231295ca9df710a2`.
- The generated Ninja graph SHA-256 is
  `4dc33b80221bdf9f9933d0fba33e207b12bfaa0009d0e7560d70263a4a8d808b`;
  `compile_commands.json` SHA-256 is
  `1b02a4bbba2963559f18c530aa8654944850ace70b7b828ce45f8d79616579e8`.
  It lists 407 compile units and no `LiGT/*.cpp`, `LiGT/*.hpp`, LiGT object,
  or `USE_PATENTED_LIGT` input. The four-target closure has 386 Ninja steps.
- Exact generated outputs are
  `build/Darwin-arm64-Release/openMVG_main_{SfMInit_ImageListing,ComputeFeatures,ComputeMatches,SfM}`;
  audited archives are `libopenMVG_multiview.a` and `liblib_CoinUtils.a` in
  that same directory. Their link/archive rules and the four
  `source/src/software/SfM/main_*.cpp` files were checked against the current
  generated graph and source before freezing the plan.

The [resume supervisor](../scripts/classical_backend/openmvg_ligt_fork_build.py)
defaults to read-only preflight. It verifies every seal, original and patched
source inventories, local pinned Eigen 3.4, the exact four-target graph and
paths, and that no build/photo attempt exists. `ninja -n` must show exactly
386 steps and no CMake regeneration before any build can start. The approved
build command would first copy the sealed `build.ninja` byte-for-byte to
`build/frozen-build.ninja`, repeat the dry-run against that frozen manifest,
then build **only** the four CLI targets at `-j 2`. This prevents Ninja from
auto-regenerating the manifest it is executing. The supervisor monitors both
generated and frozen graph hashes during the build and rechecks cache, graph,
original source/patch/Eigen seals and the one-patch fork afterward. Any
failure leaves logs and partial outputs intact; no cleanup or retry is implied.

## Resource and audit boundaries

The **entire new fork tree** is capped at 1 GiB, including source, generated
build, logs, and receipt. Before starting, external free must cover the
remaining full 1 GiB allocation **plus 11 GiB**, and both external and
internal disks must remain above 11 GiB during execution (10 GiB floor plus
1 GiB margin). Process-tree RSS is capped at 4 GiB, the build log at 16 MiB,
and build wall time at 5,400 seconds. Scratch/cache environment variables
point into the fork's external `tmp` directory. The one-shot log and manifest
paths are `logs/05-four-target-build.log` and `build-attempt-manifest.json`;
their existence blocks any rerun.

If all four exact binaries exist, the supervisor writes
`license-closure-receipt.json` with binary hashes and `otool -L` dependencies,
multiview/CoinUtils archive hashes and member lists, hashes of the four CLI
mains, and pinned cmdLine/CoinUtils/Ceres license-evidence hashes. It rejects
LiGT archive members and checks that the audited CoinUtils parser is present.
The receipt explicitly retains unresolved LGPLv3+ `cmdLine.h`, EPL Clp/Osi,
Bison exception, Ceres/Eigen, VLFeat/nonFree, and dynamic dependency questions;
it **cannot** declare the binaries shippable. See the
[license closure audit](OPENMVG-LICENSE-CLOSURE.md) for source-level evidence.

Current status: configure/graph gate passed; build **not run**. A fresh
read-only preflight and separate approval are required after any concurrent
external-disk producer has finished.
