# Architecture: a modular photo-to-mesh pipeline

CrispMesh turns photos of an object into a closed mesh. The reconstruction is
split into modules with fixed interfaces, so that each module can have several
interchangeable providers and each platform uses the ones it can run. This
document is the plan of record; status columns say what exists today.

## Stages and their interfaces

```
photos + lens calibration
   │
   ├─ masks ─────────── one object mask per photo
   ├─ cameras ───────── one pose per photo, sparse points
   ├─ undistortion ──── pinhole photos and masks
   │        ▼
   │   scene (the neutral format below)
   │        ▼
   ├─ stereo ────────── mask repair, silhouette hull, matching, fusion
   ├─ surface ───────── closed mesh
   └─ check ─────────── agreement with the photos, preview
```

Everything above the scene is provider-based. Everything below it is one
implementation, the native crate `crates/dense`. That crate is now the primary
implementation: new behaviour is developed there and judged by the scanner
evaluator on the test objects. The Python package `scripts/turntable_mesh` is
the reference for what was ported and is no longer extended.

### The scene: what providers hand to the dense stages

A directory, the same one `dense_pipeline --inputs` and `crispmesh-dense run
--inputs` take:

| File | Content |
| --- | --- |
| `cameras.json` | `{"views": [{name, source, image, mask, width, height, k, rotation, translation}], "scale": {"unit": "mm", "source": "markers"}}`; `scale` is optional and absent for a scene of arbitrary scale. `rotation`, `translation` are world-to-camera. `k = [fx, fy, cx, cy]` with half-pixel-centre principal point. Images are undistorted pinhole images. Paths are absolute or relative to the directory |
| `masks/` | 8-bit masks, object above 127, in the undistorted frame |
| `sparse_points.npy` | N×3 points; used only to locate the object |

The scene has no physical scale unless a provider says so with the `scale`
field; today the `markers` provider does (millimetres). The dense stages do not
read it yet. Handedness is that
of the photos.

### Module contracts

| Module | Input | Output | Must also report |
| --- | --- | --- | --- |
| Masks | photos | one 0/255 mask per photo, named like the photo, in the photo's own (distorted) frame | a contact sheet; photos where much of the dark region was dropped |
| Cameras | photos, lens calibration (`crispmesh_lens_calibration_v1`), optionally masks | poses for the registered photos, sparse points, the lens model actually used | registered count, reprojection statistics, orbit sanity; pass or fail against the gates in `docs/PHOTOS-TO-INPUTS.md` |
| Undistortion | photos, masks, lens model | pinhole photos and masks, updated `k` | — |

A provider is selected by name. It declares the platforms it supports; the
application offers only those. All providers write the same event log
(`docs/ENGINE-CONTRACT.md`, stages `masks` and `cameras`).

## Providers

### Cameras

| Provider | How | Desktop | Phone | Browser | Status |
| --- | --- | --- | --- | --- | --- |
| `turntable` | our own solver for ordered turntable photos with a known lens: features, tracks, bundle adjustment from a turntable initial guess | yes | yes | yes | Implemented (`--cameras turntable`; `docs/TURNTABLE-SOLVER.md`): pure Rust with its own SIFT, builds for wasm32, no random choice (two runs give identical files). Scanner F1 within 0.01 of `colmap` on four objects (+0.002, +0.001, -0.007, -0.005), above `alicevision` on all four, 13 to 24 s. Turntable captures only; verified on one camera and rig; not run in a browser. The default camera provider on every platform |
| `markers` | printed mat with fiducials on the turntable; pose from marker corners | yes | yes | yes | Implemented (`--cameras markers`, `crispmesh-dense mat`; `docs/MARKER-MAT.md`): pure Rust, builds for wasm32. Verified on rendered photos only (poses within 0.02 degrees and 0.25 mm of the truth down to 5 degrees elevation; an object reconstructed to 0.1 % of its size in millimetres). No printed mat has been photographed. Gives the scale (`scale` in `cameras.json`) and the handedness |
| `colmap` | external executable: feature extraction, sequential matching, mapper with fixed intrinsics; model converted to the scene | yes | no | no | Implemented (`--cameras colmap`). Verified on four objects through the PyCOLMAP 3.11 library with the provider's own command lines: 73 of 73 photos registered each time, scanner F1 equal to or above `alicevision`. The `colmap` executable itself has not been run (none on the development machine); COLMAP 4 option names and ring matching are untested |
| `alicevision` | external executables: features, matching, global SfM with a fixed lens | yes | no | no | Works. Native orchestration (`crispmesh-dense photos --cameras alicevision`), executables started directly from an install prefix or through a wrapper; verified on the Bunny (see `crates/dense/README.md`). The Python orchestration remains as reference |
| `import` | read an existing solution (AliceVision `.sfm`, COLMAP text or binary model) | yes | yes | yes | Works on desktop (`--cameras import:PATH`): `.sfm`, COLMAP text and binary. Reproduces the existing inputs of four objects exactly. The readers build for every target; the `photos` command is not yet built for browsers |
| `device` | poses recorded by ARKit or ARCore during capture | no | yes | no | Later, with capture |

Camera recovery by AliceVision's global SfM is not repeatable (about 0.7
degrees and 1% of the orbit radius between runs on identical features, same
reprojection error). Any provider is therefore judged by the reconstruction it
leads to: dense stages on its cameras, scanner scores on the test objects.

### Masks

| Provider | How | Desktop | Phone | Browser | Status |
| --- | --- | --- | --- | --- | --- |
| `threshold` | dark object on a light backdrop: threshold, largest component, hole cleanup; the stereo stage's multi-view repair does the rest | yes | yes | yes | Works natively (`--masks threshold`, Otsu level by default). Measured against SAM on four objects: same scanner F1 above the support (within 0.002) or higher; 0.014 to 0.019 lower over the whole surface on three of them, because the contact shadow joins the mask at the base |
| `sam` | SAM 2.1 with automatic prompts | yes | later | later | Native: `--masks sam` in builds with the cargo feature `sam-onnx` (ONNX Runtime as a shared library; the model fetched once from Hugging Face or given with `--sam-model`) or `sam-ggml` (CrispEmbed's ggml engine, a GGUF model; Metal on Apple machines). Masks equal to PyTorch on the CPU (Bunny: 69 of 73 identical, lowest IoU 0.999996). Scanner F1 within about 0.02 of `threshold` on four objects (0.912/0.966, 0.921/0.947, 0.804/0.850, 0.809/0.848 at 0.5 %, whole / above support): an option, not the default. Also through PyTorch (`--masks external-sam`, the reference script); masks it made on Apple MPS before 2026-10-06 came from a partly wrong encoder. Through ggml the masks agree as closely (CPU: 69 of 73 identical; Metal: lowest IoU 0.9991) |
| `import` | masks the user already has | yes | yes | yes | Works (`--masks import:DIR`) |

### Undistortion

One native implementation (radial `radialk3`), used after every camera provider
(`crates/dense/src/photos/scene_writer.rs`): masks by nearest sampling,
pixel-identical to OpenCV's remap; photos by bilinear sampling, on average
0.007 to 0.010 grey levels from AliceVision's `prepareDenseScene` (which
interpolates in linear light), with the Bunny's scanner F1 unchanged within
0.0012. No camera provider produces undistorted images any more.

## Dense stages

| Stage | Reference (Python) | Native (`crates/dense`) | Parity |
| --- | --- | --- | --- |
| Scene from an AliceVision solution | `dense_all_views_inputs.py` | `inputs` | identical cameras and masks on two objects |
| Stereo | `multiscale_stereo.py` | `stereo`, WebGPU compute | scanner F1 within 0.002 on four objects; about 7× faster on an M1 |
| Surface | `tsdf_hull_mesh.py` | `mesh` | within 0.002; about 4× faster, a third of the memory |
| Check | `mesh_photo_check.py` | `check` | identical scores |
| Driver | `dense_pipeline.py` | `run`, library API, C interface | synthetic scene verified; real objects in progress |

The rules are in `crates/dense/README.md`: same files, same settings,
same events, and a native stage replaces its reference only after reproducing
the scanner scores within 0.003.

## Platforms

| | macOS, Windows, Linux | iOS, Android | Browser |
| --- | --- | --- | --- |
| Compute | native crate; real objects run on Metal (Apple M1); kernels tested on Vulkan and DirectX 12 software adapters in CI | native crate through Metal, Vulkan (not yet built or measured) | WebAssembly and WebGPU: the Bunny completes in headless Chromium with the native scanner score, about 2.7 times slower than native, 1.9 GiB peak without live previews |
| Cameras | `alicevision`, `colmap`, `import`, `markers`, `turntable` | `markers`, `turntable`, `device`, `import` | `markers`, `turntable`, `import` |
| Masks | `threshold`, `import`, `sam` (ONNX Runtime, feature `sam-onnx`), `external-sam` | `threshold`, `import` | `threshold`, `import` |
| Front end | Studio (Tauri) | Studio (Tauri mobile) | Studio (static web app) |

The front end talks to the pipeline through the event log and artifact files
only (`docs/ENGINE-CONTRACT.md`), whether the pipeline runs in the same
process, in a local engine, or on another machine.

### SAM backend per platform

SAM is optional on every platform, so no build carries a SAM runtime by default.

| Platform | Recommended backend | Why |
| --- | --- | --- |
| macOS | ggml behind the feature `sam-ggml`: CrispEmbed's `libcrispembed-sam2` (2.6 MB, Metal), opened at run time, F16 GGUF (63 MB) | masks equal to PyTorch's; 1.1 to 1.5 s per photo on Metal against 2.4 s for ONNX Runtime on the CPU; a sixth of the memory |
| Windows, Linux | ggml (`sam-ggml`, CPU, or Vulkan/CUDA as the library is built) or ONNX Runtime (`sam-onnx`) | both give PyTorch's masks on the CPU; ggml needs no 33 MB runtime and less memory, ONNX Runtime is faster on the CPU (2.4 against 3.3 s per photo, measured on macOS only) |
| iOS, Android | none by default | SAM does not beat `threshold` on dark objects; if wanted, the ggml library (Metal on iOS) is the smaller route; neither built nor measured |
| Browser | none by default | `crispembed-sam2` builds for WebAssembly (1.0 MB module, SIMD) but has not been run; onnxruntime-web is the alternative |

Both backends sit behind the same provider and selection code, so the choice
is a build option and a file, not a different mask.

## Order of work

1. Native driver and library API (in progress); Studio links the crate and
   stops starting a Python engine.
2. Photos step restructured around the provider interfaces, with native
   orchestration, native undistortion of photos, `alicevision` and `threshold`
   as first providers, and the measurement of whether masks need a network.
3. `colmap` camera provider and the `import` provider, validated on the four
   test objects through the dense stages.
4. Browser build of the dense stages.
5. `markers` provider (mat design, detection, pose, scale).
6. `turntable` provider: prototype against the test objects, then Rust; the
   default camera provider since it matched `colmap` on four objects.
7. Native SAM: done through ONNX Runtime (`--masks sam`, feature `sam-onnx`); it does not beat `threshold` on
   dark objects, so it stays optional. A second backend, CrispEmbed's ggml engine (`sam-ggml`), gives the same masks with Metal and a sixth of the memory.
8. Photo upload and capture for phones; signed releases.

Development tools that stay in Python: the scanner evaluator
(`scan_evaluate.py`), the turntable diagnostic (`turntable_rig.py`) and the
synthetic test scene.

## What this does not solve

- Thin parts far from the turntable axis are still short or lost.
- Reconstruction from a dozen photos with unknown poses is not solved; all
  results use about 70 photos.
- The test objects share one camera, lens, backdrop and material.
- External camera providers bring their own licenses (AliceVision MPL-2.0,
  COLMAP BSD-3); bundling them with an application needs the same license
  review as everything else shipped.
