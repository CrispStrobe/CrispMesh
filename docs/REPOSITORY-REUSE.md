# Existing scanner repositories: targeted reuse

## 3DLiveScanner

Inspected [lvonasek/3DLiveScanner](https://github.com/lvonasek/3DLiveScanner/tree/ea057def927025418c53806339e19d3019d7dbdc) at commit `ea057def927025418c53806339e19d3019d7dbdc` on 2026-09-26. This was source/build inspection only: no clone, integration, build or hardware validation.

**Decision:** useful reference for capture, recorded datasets, texture projection and export. It is not a portable replacement for our calibrated photograph-to-depth stage.

| Area | Evidence in repository | Relevance to CrispMesh |
| --- | --- | --- |
| Mobile capture and depth | `common/arcore/arcore.cc` configures AR depth, obtains camera matrices and calls depth/confidence image APIs | Useful native mobile adapter reference; depends on platform SDK behavior and supported devices |
| Recorded sessions | `common/data/dataset.cc` reads/writes camera state, poses and point clouds; Linux dataset extractor exports recorded data | Useful replay/import design and a possible later interoperability task, not a measured golden dataset |
| Reconstruction | `common/tango/scan.cc` calls `Tango3DR_updateFromPointCloud` and mesh extraction | Central reconstruction depends on an external binary, not an exposed cross-platform photo-MVS implementation |
| Texturing | `common/postproc/texturize.cc` projects captured frames, renders depth for visibility checks, rejects frames and maps texels | Worth a focused review when mesh texturing starts; a more relevant reference than writing an untested texture pipeline from scratch |
| Surface postprocessing | `common/postproc/poisson.cc` invokes bundled PoissonRecon | Confirms an integration pattern; prefer our own pinned upstream Poisson version rather than importing the whole Android build |

The [Android build](https://github.com/lvonasek/3DLiveScanner/blob/ea057def927025418c53806339e19d3019d7dbdc/scanner/app/src/main/jni/Android.mk) links ARCore, Huawei AREngine and Tango3DR. The repository supplies Tango `.so` binaries for Android ARM ABIs, not a macOS/Windows reconstruction source backend. Its [third-party notes](https://github.com/lvonasek/3DLiveScanner/blob/ea057def927025418c53806339e19d3019d7dbdc/third_party/README.md) explicitly distinguish closed-source SDK components. Do not equate the Apache-2.0 root license—or an SDK/sample-code license label—with a completed audit of the bundled binaries and runtime terms.

The main scanner model also differs from ours. AR camera tracking is relative to a surrounding world; a fixed phone watching a rotating object needs per-frame **object-to-camera** transforms. Room-relative phone poses alone cannot align the turning object. Any RGB-D capture adapter must transform masked depths into the rotating board frame before fusion and exclude stationary background. This is a consequence of the coordinate systems, not a measured assessment of this app's accuracy.

ARCore depth does not necessarily require a physical depth sensor: it can estimate depth from motion and combine hardware depth when available. Therefore this app is not accurately described as “ToF only,” but neither does it provide us with ARCore's internal depth algorithm. See [Google's Depth documentation](https://developers.google.com/ar/develop/c/depth/developer-guide).

## How this affects the plan

- Keep Tauri and the portable computational core; study the native Android capture boundary for P02 instead of importing the Android host.
- Before R04 texturing work, compare selected `common/postproc/texturize` behavior with a dedicated texturing implementation. Audit candidate source files/dependencies before copying code and test occlusion, units and seams on controlled fixtures.
- Consider a separately scoped dataset importer only when an actual recorded 3DLiveScanner session is available. Validate timestamps, intrinsics, axes, units and depth registration; do not silently reinterpret its files as our project contract.
- If a supported phone is available, its output can become a **system-level differential comparison**, not ground truth. The current repo alone is not a runnable macOS photo-stereo oracle.

Other useful targeted references include [Open3D's RGB-D integration](https://www.open3d.org/docs/latest/tutorial/pipelines/rgbd_integration.html) for fusion once reliable depths exist, and [RTAB-Map](https://github.com/introlab/rtabmap) for recorded-data replay and RGB-D/stereo mapping. Neither removes our object-motion/masking requirement; their selected dependency graphs still need review. These are candidates, not new approved dependencies.
