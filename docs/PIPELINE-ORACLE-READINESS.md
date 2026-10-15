# COLMAP + OpenMVS research-oracle readiness

Assessment: 2026-09-26. This is a local, read-only preflight for the **same ten original tree photos** used by CrispMesh's real-tree diagnostics. No COLMAP or OpenMVS reconstruction has run; no comparator mesh, quality score, or finished-scan ranking exists. OpenMVS is AGPL-3.0 and remains a separate research executable, outside the shipped application and its dependencies. See the [comparison protocol](PIPELINE-COMPARISON.md).

Run the preflight from the repository root:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/pipeline_oracle/readiness.py --output-budget-gib 4
```

It prints JSON to stdout. The selected manifest, source records and every selected JPEG are checked against their pinned SHA-256 and byte length. It checks selected PINHOLE intrinsics and pose rows against the pinned COLMAP text export, then reports the host, disk admission, executable locations and bounded `--version` probes, and the planned stage/settings record. Version probes have a three-second timeout and 64 KiB combined output limit. The command does not create a run directory, install packages, call a remote service, or invoke a reconstruction stage. The 4 GiB output allowance is a **planning budget**, not a runtime quota; a future runner must enforce it separately. The 10 GiB free-space reserve is required *after* that allowance.

The inspected manifest SHA-256 is `c1960457e0e5f4f6d43426d2c2e047ad8354b076da5e23f06b9197402d4c2fb5`. Its ten selected photos and five source files total 202,091,069 bytes; source revision is `5f9de5e4a1be429b192a928cf1359c066dadb4b3`. The local Mac is Darwin arm64, 8 logical CPUs, 16 GiB RAM. It had 31,007,453,184 bytes free at preflight, leaving about 24.9 GiB after the proposed 4 GiB allowance, above the 10 GiB reserve. CMake, C++ compiler, Ninja, pkg-config and Homebrew are discoverable; this does **not** establish that COLMAP/OpenMVS dependencies are installed or that an arm64 build succeeds.

All six required executables (`colmap`, `InterfaceCOLMAP`, `DensifyPointCloud`, `ReconstructMesh`, `RefineMesh`, `TextureMesh`) are absent from PATH, so every stage is `unavailable` and overall readiness is `blocked`. A binary found in PATH only moves a stage to `pending_capability_verification`: a version string alone cannot prove its required CPU mode or option syntax. The supplied tree poses have no point tracks and no verified physical scale. There are no pinned object masks or independent measured geometry for a millimetre-accuracy score. The old pose export cannot be passed off as a complete OpenMVS sparse reconstruction.

## Smallest CPU-only route, when tools are supplied

1. On this Mac, obtain and pin separate research-only COLMAP and OpenMVS arm64 builds, with executable versions and hashes. Verify installed `-h` output for every chosen flag. The [COLMAP CLI](https://colmap.github.io/cli.html) documents CPU feature extraction/matching (`--FeatureExtraction.use_gpu 0`, `--FeatureMatching.use_gpu 0`), then `mapper` and `image_undistorter`. The current executable option names must be checked because versions differ.
2. Use the ten hash-verified original JPEGs as the shared input. Make a fresh, isolated work directory, then run COLMAP feature extraction, exhaustive matching, and mapping with the CPU flags. Record registered-image count and sparse point/track count before continuing. This is the **camera-estimated lane**; the supplied estimated poses are diagnostic context, not an input shortcut for it.
3. Undistort at a declared 2000-pixel maximum dimension. [OpenMVS usage](https://github.com/cdcseacave/openMVS/blob/develop/docs/wiki/Usage.md) documents `InterfaceCOLMAP` on the undistorted COLMAP model, followed by `DensifyPointCloud`, `ReconstructMesh`, optional `RefineMesh`, and `TextureMesh`. Record every installed command's settings, outputs, runtime and peak memory. A future bounded runner must stop at the 4 GiB output allowance and preserve the 10 GiB reserve.
4. Compare this output with CrispMesh on the same ten original photos using the [shared protocol](PIPELINE-COMPARISON.md). Report camera registration, successful mesh/texture production, coverage and failures. Without independent measured geometry, call geometry observations **diagnostics**, never physical accuracy or a winner claim. A fixed-camera dense lane needs a separately verified sparse scene and camera-import convention; it is not ready from this subset's trackless export.

The local Mac is the shortest route because input files, hashes and build tools are already here and its proposed storage allowance passes. The existing [VPS staging notes](REMOTE-QUALITY.md) identify Linux mounts `/mnt/storage` and `/mnt/volume1`, but there is no current VPS executable, RAM, free-space or dependency preflight for COLMAP/OpenMVS. A VPS route first needs the same script run locally on that host with the input manifest and files present, plus CPU builds and a bounded output runner. No remote upload or install is implied by this document.

Focused test command:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s scripts/pipeline_oracle -p 'test_*.py' -v
```
