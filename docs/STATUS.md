# Status

## Capture defaults, 2026-10-19

New app and native `run` requests start with a silhouette surface. Gaussian
refinement is optional; its Capture preset is 3,000 steps, a 320-pixel object
window and a 192-cell mesh grid. Stereo depth remains an explicit alternative
(`--base-surface stereo`); its dense algorithm settings are unchanged. Earlier
stereo benchmarks below describe that route, not the new silhouette default.

The optional desktop capture engine automates foreground
selection, per-view object crops, learned correspondences, focal/radial camera
recovery and coverage-aware view selection from original photos. It does not
require fixed turntable angles. It needs host-installed permissive models and
Python tools; this learned-camera route is not bundled into the iPhone engine.
The app can submit photos to it over HTTP and later add Gaussian refinement
using the cached recovered scene. New controls have English and German text.
See [capture setup](../scripts/photo_regression/README.md#automatic-real-capture-route).
Automatic foreground includes SAM refinement; importing saved preview masks
preserves their contours. Native-only masks and shared-frame cropping remain
separate options. Detection starts independently on each photo, with tracking
as a fallback and isolated-spike recovery. Camera recovery starts with a hybrid
RootSIFT overlap graph, with tight and wide alternatives. Coverage and sparse
mask support must pass. Each eligible camera path builds and checks an actual
silhouette before Gaussian training; weak agreement tries another path. Final
mask agreement is checked too.

The earlier good iPad Gaussian result also depended on mask/camera preparation
and selecting 106 of 116 recovered views. The preset alone did not reproduce
that preparation. An earlier automated raw iPad profile completed with 119
originals, 108 recovered cameras and 99 automatically retained views. Its
103,152-triangle Gaussian mesh was visually inspected and looks broadly
comparable, with weak fine relief and merged legs still visible. Successful
stage times total 1,201.9 seconds; reconstruction after recovery takes 332.2
seconds, including 230.0 seconds of training and 92.0 seconds for the hull
distance grid. This was an interrupted/resumed run: disk reserve stops required
retries. These are stage totals on a busy M1, not an uninterrupted cold benchmark
or a controlled speedup over the earlier result. Saved scenes avoid repeating
about 14 minutes of masks, matching, camera recovery and input preparation.
That profile used independent Vision detections, shared cropping and SAM;
it is evidence for that retained path, not a verification of the subsequent
app-foreground defaults. The original saved Gaussian parameters reproduce the
earlier 99,556-triangle STL byte for byte with the current mesher. Fresh training
on the same own scene and hull gives a visually comparable 99,500-triangle mesh
in 244.6 seconds (230.7 training, 13.6 meshing). This verifies reproduction after
preparation. Updated raw iPad defaults then rejected the first camera solution
(silhouette IoU 0.7735), recovered 108 cameras through the retained tight path
and produced a 103,864-triangle Gaussian mesh with the hands present. Final
all-recovered-view median mask IoU is 0.8875; Gaussian takes 273.4 seconds
(252.0 training, 20.8 meshing). Successful stage times total 1,674.1 seconds,
including fallback camera work; unchanged preprocessing was operator-adopted
and feature caches were warm, so this is not a cold benchmark. German app
result rendering and all 275 native tests pass.

The 78 original Auto iPhone photos recover 68 cameras through the wider path.
Its silhouette has 143,738 triangles and median mask IoU 0.82444; optional
Gaussian has 88,634 triangles and IoU 0.79970. The capture host keeps the passing
base as main result when optional refinement misses the unchanged 0.8 check.
Both meshes remain selectable, with an English/German explanation; Gaussian
can look smoother despite lower outline agreement. Successful stages total
1,803.1 seconds including three camera paths; Gaussian takes 238.9 seconds
(225.0 training, 13.8 meshing). Final-choice validation reused content-verified
unchanged stages after its fix, so these are not cold uninterrupted timings.
Actual final base STL has closed edges and one connected component; shape
detail remains coarse. Cached refinement uses the same full-view selector.
Both native CI builds and sphere reconstructions completed, but their old
final-surface label checks timed out. Selectors now accept the silhouette/stereo
labels; a fresh device CI run is required.

A separate camera-only second pass was tested with 84 fitting views and
22 reserved views excluded from both hulls and Gaussian training. Reserved
mask IoU changed 0.87519 → 0.87609, with no clear visual improvement; nine of
11 paired reserved RGB predictions became slightly worse. It stays optional.
Lens parameters and sparse structure were fixed; jointly refining them is a
next hypothesis. See the [camera control](../scripts/camera_refinement/README.md)
and [capture evidence](../tests/evidence/capture-app-defaults-review.json).

State of CrispMesh (engine and app) on 2026-10-19. The plan of record is
[ARCHITECTURE.md](ARCHITECTURE.md); measurements and settings are in the
[crate README](../crates/dense/README.md); the current comparison of surface
methods and the plan that follows is in [SURFACE-METHODS.md](SURFACE-METHODS.md).

## What works

| Area | State |
| --- | --- |
| **Photos to STL, one command** | `crispmesh-dense run --photos DIR --calibration JSON --output RUN`: threshold masks, our own turntable camera solver, GPU dense stages, closed STL. No Python, no external program. Historical stereo runs: about 50–100 s per 73-photo object on an Apple M1 |
| **Native crate** (`crates/dense`, Rust + wgpu) | The primary implementation. Metal, Vulkan, DirectX 12 and WebGPU; library API, C interface, event log contract; Python package kept as the reference it was ported from |
| **Browser** | The same pipeline as WebAssembly + WebGPU, from photos or from prepared inputs, single-threaded or threaded (cross-origin isolated pages). Historical stereo Bunny from 73 photos in Chrome: measured 130–264 s with 4 threads, peak WebAssembly memory about 1.6–2.1 GiB. Live at <https://crispstrobe.github.io/CrispMesh/> |
| **CrispMesh** (app) | Desktop (macOS, Windows, Linux), iOS and Android from one code base (Tauri 2); the engine runs in-process. Start from photos with provider choice, live progress, preview surfaces, diagnostic sheets, per-step inspection sheets, example objects downloaded on request. Prereleases (unsigned desktop builds) on the [releases page](https://github.com/CrispStrobe/CrispMesh/releases); internal TestFlight builds for iOS and macOS |
| **Taking photos** | Live camera, manual or Auto shutter, 1–4× digital zoom before the first photo, selection/deletion and renumbering, pause/Continue and up to 16 camera heights with unequal photo counts. Capacity raised to 1,536 photos total (resource limits still apply). Auto compares the saved frame, compensates moderate exposure changes, checks settling drift and a minimum interval; tap the object to choose the watched area. English/German phone-layout checks pass; updated real-device Auto validation is pending |
| **Few photos** | Historical neural-surface tests on textured objects: Bunny 0.839, Armadillo 0.852 at 0.5 % from 19 photos; the ordinary stereo mesh 0.68 / 0.70. New runs now retain the silhouette default and leave refinement optional. Limited coverage is explained in the app; sparse-camera recovery can still refuse a set. [PHOTOS-TO-INPUTS.md](PHOTOS-TO-INPUTS.md) "About 20 photos" |
| **Languages** | Studio-composed texts, including Take photos, Auto, New run and few-photo guidance, have English and German catalogues. Apple permission-prompt resources and all 74 dense setting labels/descriptions include both languages. Other engine-provided descriptions, events and errors still need localization |
| **No lens file** | `--calibration auto`: the focal length from the closed turn (raw steps scale with it). Five of six 3DLF objects within 3.3 %, F1 within 0.006; the Happy Buddha 28 % off. `--calibration fov:DEG` (a known field of view; *Take photos* in the iOS app asks the system): within 5 % as good as the lens file on Bunny, Dragon, Happy Buddha (Buddha +0.016 F1 over automatic); a field of view that does not close the turn falls back to automatic. [PHOTOS-TO-INPUTS.md](PHOTOS-TO-INPUTS.md) |
| **Phone to computer** | The desktop app lets other devices use its engine (local network, token, QR pairing, `POST /api/uploads`); the phone sends its photos and follows the run. The desktop app also announces itself (Bonjour `_crispmesh._tcp`, macOS/iOS) and shows a six-digit pairing code, so a phone can find it without the QR code. Checked in Chrome against the app's server and with an mDNS self-test on the Mac; on devices in TestFlight build 13; [ENGINE-CONTRACT.md](ENGINE-CONTRACT.md) |
| **Masks** | `threshold` (default; contact shadow removed), `import`, `sam` (SAM 2.1 natively through ONNX Runtime or CrispEmbed's ggml engine, optional build feature), `external-sam` (PyTorch) |
| **Cameras** | `turntable` (default; our own solver, pure Rust, repeatable), `markers` (printed mat, gives scale and handedness; validated on renders only), `colmap`, `alicevision`, `import`; `--capture orbit` for camera paths that are not one ring |
| **Evaluation** | Scanner F1 against independent scans (3DLF), DTU protocol, Google Scanned Objects rendered by our own renderer (`crispmesh-dense render`), per-step inspection sheets in every run |

## Historical measured results (stereo route, from photos)

F1 at 0.5 % of the scan diagonal, whole surface / above the support; scans used
for scoring only. Exact numbers per threshold and per commit are in the crate
README.

| Object (3DLF, 73 photos) | F1 at 0.5 % |
| --- | --- |
| Bunny | about 0.90 / 0.96 |
| Armadillo | about 0.92 / 0.95 |
| Dragon | about 0.80 / 0.85 |
| Lucy | about 0.85 / 0.88 |
| Thai statue | about 0.88 / 0.95 |
| Happy Buddha | about 0.79 / 0.83 |

| Other sets | Result |
| --- | --- |
| Google Scanned Objects renders (72 photos) | rhino 0.867, cereal box 0.833 at 0.5 % with threshold masks; light-coloured objects need better masks (Mario 0.71 → 0.83 with exact masks) |
| DTU historical dense-stage diagnostics (49 views, supplied masks; supplied or COLMAP poses) | 1.03–1.31 mm mean of accuracy and completeness on scans 63 and 65; these are not photo-only pipeline results |
| YCB turntable | cracker box 0.45 at 0.5 %; mustard bottle from the DSLR photos 0.73 at 0.5 % with one turn, 0.48 with five before, 0.77 with five now (silhouette first) |

## Findings worth knowing

- The 3DLF scans are mirror images of the photographed objects; our
  reconstructions have correct handedness (shown with an asymmetric synthetic
  object with known truth).
- PyTorch 2.7 on Apple MPS computes SAM 2.1's Hiera encoder wrongly (a strided
  `max_pool2d`); masks made that way before the fix were partly wrong.
- With correct arithmetic, SAM masks do not beat the threshold masks on this
  capture style; threshold is the default.
- The 3DLF light-field photos are soft (most image energy below an eighth of
  the sampling frequency). Fine relief is nevertheless visible in them and is
  poorly recovered. Per-pixel slanted-plane matching (PatchMatch-style,
  now excluded from ordinary builds pending patent review) reduces noise and improves some folds, but
  close-ups still show weak Bunny eyes, nose and head relief. Whole-object F1
  gains do not establish recovery of those features. Follow-up matching,
  fusion and smoothing experiments are recorded in
  [bunny-head-detail-review.json](../tests/evidence/bunny-head-detail-review.json);
  none was adopted as a default.
- Where the drill's error comes from (2026-10-10, own photo-derived inputs;
  the scan only for scoring): fusion and meshing reconstruct every seen part
  almost exactly from exact depths (the unseen underside is the remaining
  loss), and dropping our depths where the true surface is not seen changes
  little, so neither fusion, meshing nor coverage is the bottleneck. The depth
  maps scatter about 0.2 % of the distance; with the true surface shape the
  photo-consistency peak itself scatters as much, so at this resolution the
  photos, not the matcher, set that limit. What was lost was averaging: a 3-voxel
  truncation clipped depths that scatter by 3 voxels instead of averaging them.
  Fusion now widens the truncation to three times the noise measured between
  the views' own depth maps (`truncation_noise=3`): drill F1 at 0.5 % 0.36 →
  0.49 with production defaults from the same depths, six-object gate passed
  (worst change −0.0013). Creases, vents and knurling of about a millimetre
  remain below what these photos resolve. See
  [noise-matched-truncation-review.json](../tests/evidence/noise-matched-truncation-review.json).
- The drill's limit was the photos, and better photos fix most of it (2026-10-11).
  The same pipeline on the YCB capture's DSLR photos (4272 x 2848, 3.3 times the
  pixels per millimetre) scores F1 at 0.5 % 0.69 instead of 0.49 with mean error
  0.0053 instead of 0.0093; with all five DSLR camera heights (300 photos, new
  `--turntable-rings 5`) the openings under the motor housing and along the handle
  close and F1 at 0.5 % reaches 0.72 (scan aligned by photo-consistency; 0.80 with
  mesh ICP). Surfaces stay mildly lumpy and vents, knurling and lettering are not
  recovered. Large photos needed a camera-solver working resolution
  (`--turntable-working-size`, 2200 px) and bounded grey-image memory; captures up
  to 2200 px are unchanged byte for byte. See
  [ycb-drill-dslr-rings-review.json](../tests/evidence/ycb-drill-dslr-rings-review.json)
  and the turntable solver document. Capture advice that follows: fill the frame
  with the object and add camera heights rather than more photos per turn.
- Several turns are more robust and do not always help (2026-10-11). A turn
  whose own solution fails no longer stops the run: its photos are registered
  against the other turns and kept on the turntable motion through the bundle
  adjustment. Default deadlines scale with the number of turns, and each turn's
  photos are audited on their own. The YCB mustard bottle's five DSLR heights
  register 300 of 300, but the top-down turn's depths are 1.5 % too far and the
  mesh (F1 at 0.5 % 0.48) is worse than one side turn (0.73). The drill's
  five turns are unchanged (0.72), and single-turn captures are byte-identical.
  See [ycb-mustard-rings-review.json](../tests/evidence/ycb-mustard-rings-review.json).
- Silhouette first for several camera heights (2026-10-12). With five
  heights the silhouette hull is already close to the object, and noisy depth
  mostly digs pits into it. Three settings: depths count only where views from
  clearly different directions confirm them (`cross_check`), masks are grown
  where confirmed depths land outside them and the run restarts
  (`mask_grow`), and the surface is the hull moved inward only where the
  averaged depth offsets agree (`mesh_hull_correction=4`). The mustard bottle's
  masks had left out its pale label, which made the hull 7.5 % too thin and,
  through the hull, misplaced the depths: a main cause of the poor five-turn result. One command each,
  F1 at 0.5 / 1 / 2 %: mustard 0.48 / 0.74 / 0.87 → 0.77 / 0.92 / 1.00, drill
  0.80 / 0.89 / 0.94 → 0.82 / 0.92 / 1.00, both closed and smooth, fine relief
  still missing. Single-turn captures need the depth as the surface (the six
  3DLF objects lose 0.12 to 0.37 with the correction), so `silhouette_first`
  (default 2) applies the three steps only when the cameras look at the object
  from several heights all the way round; every single-ring and one-sided
  capture checked (36 camera sets) is left as it was, byte for byte. It costs
  half as much time again to twice the time on those captures. In a browser
  the dense stages ran on a reduced drill scene (100 views): 218 s and
  2.44 GiB of WebAssembly memory against 141 s and 2.28 GiB without, closed
  against open. From photos, the five turns (300 photos at 1424 × 949) run
  through in a browser in 760 s with 3.2 GiB to a closed mesh (0.74 / 0.88 /
  1.00); at 2136 × 1424 the photos stage's memory leaves no room for the
  dense stage.
  See [silhouette-first-review.json](../tests/evidence/silhouette-first-review.json).
- At 10° steps (36 photos) results are almost as good as at 5°; at 15° steps
  low-texture objects fail camera recovery.
- About 20 photos (19° steps) work with the neural surface where the cameras
  are found: Bunny 0.839 / 0.917 / 0.948 and Armadillo 0.852 / 0.949 / 0.977
  (ordinary mesh 0.676 and 0.696 at 0.5 %). Below 25 photos per turn the
  landmark gate is 8 instead of 20; the ring checks refuse Dragon and Lucy at
  19 photos and the Bunny at 15 (docs/PHOTOS-TO-INPUTS.md, "About 20 photos").

## In progress

- **Real-user capture work (2026-10-18):** photo selection/deletion, unequal capture turns,
  zoom, and capture/New run German texts are implemented. Optional foreground crop previews
  on macOS 14 / iOS 17 show three photos with padding, search-area and mask-threshold controls;
  masks follow the previous object region, and reviewed sample masks are reused on saving.
  Cropping preserves image sampling and shifts the camera principal point. Browser and other
  platforms keep ordinary capture; this foreground provider is Apple-only.
- **Still open:** a general in-process camera solver for arbitrary hand-held paths; reliable
  meshes from real phone batches; updated iPhone Auto and crop validation; localization of
  engine-generated text. Increasing the input limit is not a 1,536-photo device benchmark.
- **Private real-user batches:** 119 manually triggered iPad photos and 78 Auto iPhone photos
  were copied through USB without changing the device files. Own foreground segmentation
  and crop controls are promising, but the iPad photo-only control (105 registered cameras)
  produced a distorted mesh; a silhouette-only control is also distorted. Neither is a
  suitable printable reconstruction. No ground-truth masks, poses, depths or meshes were used.
  The iPhone contains confirmed near-repeated consecutive viewpoints (1–2, 3–4, 32–33);
  the programmed LEGO angle is unknown. An all-78-photo control reduced to 1,600 pixels
  was refused by the native camera order gate (neighbour/four-apart match medians 4/4).
  Saved-photo differences cannot establish physical
  step angles or distinguish motor timing from shutter timing. No uniform-spacing claim.
  An experimental angle-independent Rust library solver recovers only 21 of 117 accepted
  iPad photos before robust track filtering, 28 afterward; the same filter raises native
  iPhone recovery from 8/78 to 11/78. An explicit BSD-3-Clause P3P alternative
  reduces measured solve times from 258 to 30 seconds on iPhone and 143 to 55 on iPad,
  but registers only 11/78 and 27/117 respectively. These are single controls, not
  a performance guarantee; the original linear path remains available and default.
  It is not exposed as a provider. Synthetic rotation and shape tests pass,
  but this real-batch coverage fails. Fixed-lens iPhone controls improve from at most 18 cameras with 382 selected pairs
  to 61 of 78 with exhaustive matching (3,003 pairs); coverage still fails, and the
  secondary component does not align consistently. Joint sparse-feature/silhouette fitting
  improves fitted mask support but a dense diagnostic on that 61-photo subset remains
  distorted and open (119,133 triangles, genus 14). No suitable STL yet.
  Reviewing all 78 masks found a table selection on photo 66 that the earlier sampled
  review missed. A new batch check retries isolated oversized masks inside neighbouring
  object regions; native replay selects the figure, with some shadow still included.
  Unresolved originals are retained for review. Radial-lens controls reach 63/78 cameras,
  but silhouette agreement remains poor. A free-angle shared-axis diagnostic also fails
  feature accuracy; it is not adopted.
  Private Apache MapAnything four-view controls also remain flattened; the 78-view
  336-pixel prototype is memory-limited on MPS, even after exact attention chunking.
  Lower-resolution chunked 12- and 78-view runs finish but produce constant depth and
  invalid focal estimates; they are rejected before geometry assessment.
  Existing splat/surface trainers keep recovered cameras fixed; they cannot repair
  these poses automatically. See [camera alternatives](SURFACE-METHODS.md#camera-recovery-and-gaussian-alternatives-on-irregular-phone-captures-2026-10-18).
  Evidence: [real-user-capture-review.json](../tests/evidence/real-user-capture-review.json).

- **Surface quality on captures from several heights** (2026-10-13). Like for
  like on the same masks and hull, fused depth is worth 0.12 F1 at 0.5 % on the
  textured drill and nothing on the plain mustard bottle, at several times the
  run time of the silhouette alone. Refining the mesh directly against the
  photos, without depth maps, is now a pipeline stage (`photo_refine`,
  automatic for these captures): drill 0.82 / 0.92 / 1.00 → 0.86 / 0.98 / 1.00
  with no settings, the mustard bottle unchanged within the fit thanks to a
  distinct-peak acceptance test. Gaussian splatting (Brush) renders held-out photos sharply from our
  cameras and gives a less accurate surface than our meshes. Reference runs on
  a GPU (2D Gaussian splatting with gsplat, NeuS-facto in SDFStudio) score
  like ours (drill 0.88 and 0.89 at 0.5 % against our 0.86) and look far
  better: the neural surface shows vents, ridges and seams that our meshes
  lack. A surface optimised as a whole is the quality target from here.
  [Measurements, comparison with other pipelines and plan](SURFACE-METHODS.md).

- **Photo-textured GLB**: opt-in `run --texture` and standalone `texture`
  command, also offered in Studio for built-in engines. An embedded UV atlas
  uses the pipeline's own recovered cameras, photos and masks; unseen regions
  are grey. Geometry is preserved. Initial view-coherence/visibility checks
  reduce patchwork, but seams and captured-lighting differences remain.
  Studio now previews GLB on request and switches to grey on the same geometry;
  preview loading, closing, retry and cancellation were browser-tested.
  Full browser reconstruction/texturing memory and real-device sharing
  still need validation. Fresh Rhino: 72/72 photos, 96 s including initial
  texturing; mustard stops before stereo because threshold masks select the
  undistortion border. [Usage and limits](TEXTURED-MESH.md),
  [photo-only evidence](../tests/evidence/photo-texture-review.json).

- **Facial detail and source review**: optional neighboring-plane PatchMatch
  is excluded from ordinary engine builds and settings forms. The explicit
  `research-patchmatch` Cargo feature retains it for research, without granting
  patent rights. A targeted [source and patent screen](../tests/evidence/mvs-source-review.json)
  records the relevant claims and the limits of the review; it is not legal
  clearance. The default band pipeline remains unchanged.
  Independent per-pixel plane refinement is implemented as an experiment
  (`slanted_refine=false` by default), with no neighboring-plane candidates or
  random perturbation. Historical saved Bunny eye/brow-region F1 at 0.1% of the object diagonal
  rises from 0.579 to 0.593, below the earlier research PatchMatch result of
  0.609. Close-ups still show weak eyes and nose. Supplied scanner geometry is
  used only for posthoc evaluation, never reconstruction.
  Optional subpixel fusion (`fusion_interpolate=false`) raises the combined
  local eye-region F1 to 0.601. Neither experiment has passed the six-object
  adoption gate; both stay off. [Evidence and limits](../tests/evidence/bunny-independent-detail-review.json).
- **Example objects in the app** from several sources (CC BY renders first).
  The rhino passed the sandboxed Mac app check (72 registered photos).
  Screenshot capture now selects the viewer's upright direction for this
  object; the corrected iOS/Mac screenshot dry run and Pages check remain
  pending CI. No screenshots have been uploaded to App Store Connect.

## Known limits

- Masks assume a dark object on a light backdrop; light-coloured objects and
  dark backdrops need SAM or imported masks. Threshold masks covering over
  90% of every photo are refused before camera recovery, with a mask sheet
  kept for inspection.
- Interiors a single ring of cameras never sees (mugs, shoes) are capped.
- Thin, low-texture parts and very smooth objects remain hard.
- Plain single-colour surfaces give photo-based depth nothing to agree on:
  there the silhouette from several camera heights is the best surface we have.
- The multi-height photos stage needs about 13 MB of WebAssembly memory per
  photo in a browser; 300 photos work at 1424 x 949 and not at 2136 x 1424.
- The marker mat has not been tested with a real print.
- Builds have been run on Apple hardware only (desktop app, iOS simulator,
  browser); Windows, Linux and Android builds are tested in CI only. The owner
  has started trying TestFlight builds on an iPhone (2026-10-18).

## Owner's open requests

1. Validate Auto on the real LEGO turntable with internal TestFlight 0.3.0 (15):
   missed steps, duplicate photos, zoom, deletion and the foreground crop preview.
   Both Apple uploads and internal-group assignments completed; device results are pending.
2. Recover arbitrary handheld paths and irregular or repeated turntable angles in-process.
   The configured desktop host now offers learned recovery to the app without angle
   inputs. The experimental in-process library solver is not yet an app/CLI provider.
3. Improve camera alignment and fine geometry beyond the automatically cropped coarse
   iPad/iPhone results above. The earlier Gaussian result is reproducible; the first
   silhouette-guided camera-only pass gave no clear improvement on reserved views.
4. Translate all remaining engine-provided user-facing descriptions, events and errors
   into German. Capture, dense settings and surface-method controls are translated.
5. Validate substantially larger batches on devices and in browsers, including scalable
   camera adjustment. The native input limit is 1,536; resource limits still apply.
6. Validate several table turns with the camera moving up/down between them.
   Capture supports up to 16 height groups with unequal counts; automatic hardware
   coordination and reliable reconstruction of real multi-height captures remain open.
7. Move a validated build to external TestFlight only after the owner's device test
   and decision. External testing remains on its previous build.

## Data published for this project

| Repository | Content | License |
| --- | --- | --- |
| [cstr/3dlf-scan-photos](https://huggingface.co/datasets/cstr/3dlf-scan-photos) | 3DLF-Scan turntable photos of seven objects (the app's example objects) | CC BY 4.0 |
| [cstr/3dlf-scan](https://huggingface.co/datasets/cstr/3dlf-scan) | the full 3DLF-Scan release, extracted (without the canonical Stanford model files) | CC BY 4.0 |
| [cstr/gso-turntable-photos](https://huggingface.co/datasets/cstr/gso-turntable-photos) | rendered turntable photos of Google Scanned Objects | CC BY 4.0 |
| [cstr/sam2.1-hiera-tiny-ONNX](https://huggingface.co/cstr/sam2.1-hiera-tiny-ONNX) | SAM 2.1 Hiera-tiny, ONNX export | Apache-2.0 |
| [cstr/sam2.1-hiera-tiny-GGUF](https://huggingface.co/cstr/sam2.1-hiera-tiny-GGUF) | SAM 2.1 Hiera-tiny for CrispEmbed's ggml engine (F16, F32) | Apache-2.0 |

Photo robustness follow-up (2026-10-08): opt-in background-colour masks and an
experimental rotating-support camera initializer; defaults unchanged. Shoe
and Mario produce STL and GLB with identical triangle geometry. Mario with
photo-derived native SAM masks scores F1 0.830 at 0.5% of reference diagonal;
shoe gains are modest. Mustard now registers with photo-derived masks and
surface features, but its wrinkled mesh fails visual review. It is not fixed.
See [other image sets](OTHER-IMAGE-SETS.md) and the
[regression runner](../scripts/photo_regression/README.md).
A silhouette-only mustard control yields a useful coarse closed bottle and GLB
(F1 0.637/0.836/0.890), versus 0.181/0.391/0.658 for its damaged stereo mesh.
The current dense evidence damages the coarse hull; this does not distinguish
camera error, matching, filtering or fusion. The approximation still loses
neck/spout geometry and unseen concavities.
Cereal box: 72/72 recovered cameras and both exports; background masks retain
the disk, while SAM removes it but misses pale edges. Printed lines still become
false grooves. A local combined-mask/silhouette prototype is closed genus 0,
but facets and texture defects remain; it is not a shipped/general fix. Drill
is still refused by camera gates. Evaluation warnings no longer infer which
input is mirrored from a preferred reflected shape fit alone.


Quality follow-up (2026-10-08): source-photo annotations corrected partly
offset eye/nose depth boxes. New stage profiles report coverage and errors
on common rays, with scanner geometry used only for evaluation. In one nose
view, independent-plane depth coverage falls from 93.7% before cross-view
filtering to 58.2% afterwards; weak/noisy relief is already present before that
filter. This does not isolate cameras, matching and appearance errors.
Adaptive spatial/intensity support takes 248 s of refinement versus 75 s for
the control, without convincing recovery of the eyes/nose. Geometric-confidence
fusion also leaves the difficult structural failures unresolved. Those dense
experiments were not adopted; the original matcher was retained after its
shortcut showed no reliable speed gain.

New standalone texture options `--coherent --color-balance` preserve every
triangle coordinate and winding byte in five reviewed exports. Source-view
boundaries decrease about 12–17%; visible seams remain. Twenty-four views
reduce some grey seams on the cereal approximation, but do not fix its shape,
and increase atlas pixels 56%. Both new controls and SAM's experimental
`--sam-prompt-mask background` stay opt-in. On drill, multiple-candidate SAM
finishes all 60 masks but includes support/background in several views; camera
recovery still fails its unchanged support gate (24.8%, required 25%).
No drill mesh is accepted. Reserved-track mustard camera diagnostics and
all experiment limits are in the [quality review](../tests/evidence/quality-followup-review.json)
and [diagnostic usage](../scripts/photo_regression/README.md).


Drill continuation: enabling existing automatic SAM cues with background
prompts and several candidates now recovers 60/60 cameras from the same
cached own RGB matches, with the 25% support gate unchanged (25.42% support).
The first textured GLB renders correctly, but its grey mesh is pitted/open
and fails the detail target. Independent planes and a 12-degree minimum
neighbor-angle control do not resolve the openings; no default changed.
See [drill controls](../tests/evidence/drill-cues-and-detail-controls.json) and
[other image sets](OTHER-IMAGE-SETS.md#drill-recovery-control-with-automatic-cues).


Drill failure isolation: a fresh baseline reproduces the STL byte for byte.
Raw finest depth estimates already omit broad casing regions and have
irregular slopes. Finest consistent median coverage is 51.4%; coarse fallback
raises it to 71.5%. A diagnostic mesh using only the existing silhouette prior
(no accepted stereo weights, unseen closure enabled, overshoot disabled)
keeps a recognizable coarse shape without the deep pits. It still misses real
surface detail and is not an accepted reconstruction. This points to unreliable
stereo/fusion evidence as a major contributor rather than texture export alone.
The held-out camera subset has 58 foreground observations, median/p95
0.445/1.501 px; the full foreground set is much worse, and background matches
include many outliers. Cycle-confirmed training matches do not improve the
full foreground holdout and were not adopted. Photo-only mask grouping is now
available in the camera diagnostic. See the
[depth failure evidence](../tests/evidence/drill-depth-failure-diagnostic.json).


### Original texture RGB and drill controls (2026-10-08)

New photo-stage runs keep matching contrast separate from captured colour:
`texture_image` names the original RGB, undistorted with the same recovered
lens. Legacy scenes still fall back to their matching image. A 60-view native
control preserved every camera parameter and matching-image byte while its
texture pixels matched the original captures. The GLB preserved all 1,006,012
triangle coordinates and winding, and passed format validation. The full
threaded browser run passed too; see [texture evidence](../tests/evidence/original-texture-rgb.json).
This fixes the colour source; the drill's geometry remains unacceptable.

Original-RGB matching, frozen-prior geometric cost, reduced free-space carving,
a full native sweep and Poisson reconstruction from our own filtered depths did
not meet the drill shape target. Object-only COLMAP registered 44/60 and failed
the existing camera gate; unmasked COLMAP could not initialize a model. No gate
was relaxed and no dense default changed. [Recorded controls](../tests/evidence/drill-matching-controls.json)
retain the rendered comparison paths and limitations; these are negative
results, not an accepted quality improvement or six-object adoption validation.

### Depth reliability experiments (2026-10-09)

A private Apache-2.0 DA3-Small experiment produced smoother drill surfaces,
but reduced strict reference F1 from 0.386 to 0.300 at 0.5% of the reference
diagonal. The reference was used only for posthoc evaluation. Cross-view
agreement and smooth appearance did not establish correct shape; the learned
prior and its refinement controls were rejected. No model code, weights or
new dependency was shipped. See the [learned-depth review](../tests/evidence/learned-depth-drill-review.json)
for pinned sources, resource limits and evaluation qualifications.

A first NCC uniqueness filter improved drill F1 but severely damaged Bunny,
including its eye region. It confused broad single peaks with competing
depths and failed the default adoption gate. It was rejected; see the
[confidence review](../tests/evidence/peak-confidence-review.json).
Dense defaults remain unchanged.

The revised experimental `peak_min_margin=0.02` requires two distinct local
maxima separated by a real score valley, preserving broad single peaks.
All six RGB-derived object pairs pass the 0.003 F1 regression gate, with paired
renders reviewed. Drill strict F1 improves 0.386 to 0.409; Bunny's tight eye
region changes 0.582 to 0.586, without convincing eyes/nose recovery. The
portable native implementation reproduces both experimental STLs byte for
byte, and its disabled control reproduces the original drill STL. This is an
opt-in reliability experiment, not an accepted drill reconstruction.
The threaded browser executes all 60 photos through cameras, stereo, mesh,
check and textured GLB export with this setting: 52.7 seconds and 1.07 GiB
WASM high water in one local Chrome/Metal run with two workers. These are
execution measurements, not browser/GPU RSS or a controlled speed comparison.

Separate posthoc comparison with supplied YCB camera poses measures median
0.376-degree rotation error relative to the first recovered camera, and 4.0 mm
translation residual after one scale fit. Adjacent-view errors are smaller
(median 0.079 degrees / 1.17 mm). The supplied calibration has its own error;
these measurements do not establish which error causes the bad depths.
Projecting our own cameras onto a rigid ring improves some neighbouring-pose
statistics but worsens independent held-out foreground photo residuals; it
was rejected. Supplied poses never entered reconstruction or the ring fit.
See [distinct-mode and camera evidence](../tests/evidence/distinct-depth-modes-review.json).
The final rigid-ring mesh control changes strict F1 only 0.409 to 0.412,
with extensive visible defects remaining. A separate control removes grazing
depth normals beyond 60 degrees (333,764 of 1,114,571 valid depth pixels),
then repeats fusion on the same own inputs; it also leaves extensive defects.
Neither control is adopted. No reference F1 or six-object gate was measured
for the grazing-normal control.

Subpixel refinement of own drill feature matches was rejected: on a fixed
held-out foreground subset, median residual changes 0.445 to 0.422 pixels,
but p95 worsens 1.501 to 1.542 pixels and the full foreground median worsens
1.000 to 1.023 pixels. The camera diagnostic now supports fixed eligibility
from a separate own baseline solve and reports invalid selected observations.
Both solves must exclude the same reserved training components. Five tests
cover selection independence, view ordering and invalid observations.
See [camera subpixel review](../tests/evidence/camera-subpixel-review.json).

A private source-occlusion control rejects samples lying more than 0.5%
behind our previous-pass source depth. Separately fitted drill strict F1 changes 0.409 to
0.422, but the fixed-alignment follow-up below does not confirm that gain;
pits, false relief and an open mesh persist. Bunny passes
its numerical regression check; eye-region F1 changes 0.586 to 0.594 while
mean surface error worsens, and the renders do not show convincing eye/nose
recovery. Seven synthetic GPU cases pass and the disabled drill control
reproduces all final depths and the STL exactly. This control remains private:
no production integration, five other object checks or browser execution.
See [source visibility evidence](../tests/evidence/source-visibility-review.json).

Fixed-alignment follow-up corrects the interpretation of that visibility gain:
with one baseline scan fit and 200,000 samples per surface, strict drill F1
is 0.4108 for distinct modes, 0.4089 for visibility and 0.4097 for a new
conservative visibility control. The latter uses only accepted depths with
full coherent 3x3 support, never hole-filled initial surfaces; it worsens
all three drill thresholds and mean error. Both visibility variants are
rejected for integration. Bunny's conservative variant has only tiny local
changes, without convincing eye/nose restoration. Its prior-validity test
passes; no other-object or browser gate was run for these rejected controls.

A stronger drill lead is confidence for coarse fallback. On the same own depths,
replaying finest accepted depths alone changes fixed-alignment F1 at
0.5/1/2% from 0.411/0.637/0.821 to 0.453/0.699/0.925. Replaying merged
depths reproduces the baseline STL byte for byte. A private control keeps
coarse gap fills only when three finest-depth voters agree, preserving all
fine pixels and retaining 45,497 of 295,747 fallback candidates. Its F1 is
0.456/0.702/0.929 and symmetric mean surface error is about 24% lower than
ordinary fallback. Three identical-view renders were inspected. The meshes
remain open and visibly defective; this is a useful reliability lead, not an
accepted drill reconstruction. No default or app setting changes were made;
other-object validation, native parity and browser execution remain required.
See [supported fallback evidence](../tests/evidence/supported-fallback-review.json).

The supported-fallback STL also has a locally inspected original-RGB GLB;
all triangle coordinates/winding are byte-equal, source attribution is embedded,
and Khronos validation reports zero errors/warnings/infos/hints. About one
third of surface area remains untextured; seams and geometric defects persist.
This is standalone export/render validation, not browser pipeline adoption.

Supported fallback is now implemented in both engines as an opt-in setting,
`fallback_supported=true` (default false). It preserves accepted fine pixels
at the merge and requires added coarse fills to agree with finest maps in
other views, using the final configured tolerance/minimum votes even when
native resolution ends the pyramid early. Self/duplicate voters cannot count.
Native disabled drill depths and STL remain byte-identical. The port accepts
two extra pixels relative to NumPy near numerical boundaries; its fixed-fit
drill F1 is 0.455/0.701/0.929, with about 24% lower mean error than ordinary
fallback, but the mesh remains open and defective.

All six same-input pairs with `fallback_supported=true, peak_min_margin=0.02`
pass the 0.003 F1 regression rule (whole and above support, 0.5/1/2%); all
paired sheets were inspected. Worst drop is 0.00279 on Happy Buddha.
Bunny eye/brow strict F1 improves 0.582 to 0.613 and local mean error falls
about 8%, but missing eye/nose relief is not convincingly restored. Both
settings remain off by default. The final native suite passes 201 tests,
Studio settings 18; Studio and threaded WASM builds pass. A full 60-photo
browser run confirms supported fallback executes and exports original-RGB
GLB: 75.5 seconds, 1.24 GiB WASM high water with two workers. These are one-run
execution measurements, not a controlled speed comparison or total browser
memory. An earlier browser attempt stopped at the 8.8 GiB disk floor; verified
SSD moves restored about 12 GiB before the successful retry.


Follow-up reliability controls (2026-10-09) did not justify another adoption.
A private cross-view depth correction preserved coverage and coarse fills,
used only agreeing immutable fine maps, and reproduced all 60 drill maps
exactly at strength zero. Drill F1 worsened from 0.455286/0.701277/0.929251
to 0.454555/0.699303/0.928121. Bunny strict eye/brow F1 rose 0.612953 to
0.618229, but the finest measured eye/nose relief error worsened in all five
crops; paired renders did not restore the missing features. This control
remains private and rejected as a general change.

Replaying the same supported depths with existing `fusion_interpolate=true`
made negligible drill changes; Bunny mean head error worsened despite a small
F1 increase. Its existing opt-in status is unchanged. A separate constrained
camera bundle control reduced training median reprojection 0.288 to 0.261 px,
but fixed reserved foreground errors worsened: median 0.445 to 0.513 px,
p95 1.501 to 1.521 px (58 observations, none invalid). No dense run used those
candidate cameras. Eight camera-holdout/profile diagnostic tests pass.
No production algorithm or default changed in these follow-ups; rejected
controls were not taken through six-object/browser adoption gates. See
[detail reliability controls](../tests/evidence/detail-reliability-controls-review.json).
The remaining detail problem needs better local photo correspondence and
view support; these results do not establish a single root cause.


Source-selection controls (2026-10-09) remain private. Holding the initial
surface's best reliable source views fixed across each band search improves
Bunny strict eye/brow F1 0.613 to 0.637 and mean head error about 6%, but all
three drill F1 thresholds worsen, including 2% F1 0.929 to 0.898. The smaller
hole count does not imply a better shape. A .02 preference penalty instead of
strict locking also worsens drill and gives no Bunny head improvement.

A further control locks views only after an ordinary finest-depth pass.
First-pass raw/filtered arrays match its ordinary control exactly. Drill needs
no additional pass at its native resolution; Bunny uses two finest passes in
both comparison arms. This also worsens drill; Bunny head F1 is 0.605 for
ordinary extra refinement and 0.605 with view selection, below 0.613 baseline.
All paired sheets and five local eye/nose depth profiles were reviewed. Two
CPU/GPU matching parity checks pass; disabling the initial experiment produces
exact baseline drill depths/STL. The production matcher, shader and level driver
are restored, and the shared native CLI is rebuilt from that original source.
No defaults, production policy or dependency changed; no six-object/browser
adoption gate was attempted for these rejected general controls. See
[source-selection evidence](../tests/evidence/source-selection-review.json).

An independent [depth-quality audit](DEPTH-QUALITY-AUDIT.md) now reproduces a
fundamental band-matching failure with perfect cameras: a correct depth 2.000
becomes 1.895 solely because neighbouring initial depths are biased. Removing
cost aggregation does not cure it. Coherent centre-plane scoring keeps 2.000
and recovers the damaged band result to 1.999 in this controlled case. Exact
analytic depths through production fusion/meshing preserve a recess and raised
feature, with sampled errors below one voxel; pinhole ray round trips pass.
Three release audit tests pass with Metal enabled. These findings identify a
specific matching-support defect, not the complete cause of real-object errors
or a default-adoption result. The coherent matching correction is recorded
below. COLMAP, OpenMVS and Open3D primary implementations were compared without incorporating upstream algorithm code. See
[audit evidence](../tests/evidence/depth-geometry-audit.json).


An experimental `coherent_band=true` now scores a single centre depth/normal
surface in every finer-level and hull-front band search. It corrects the
perfect-camera neighbouring-prior failure and improves Bunny head F1 .612953
→ .641016 against the same supported-fallback baseline (mean head error about
9.9% lower). Drill F1 .455286/.701277/.929251 → .481374/.720322/.940236 at
.5/1/2% with the same recovered inputs and frozen evaluation alignment. Eyes,
nose and drill geometry remain inadequate in inspected sheets. Lucy exceeds
the default regression budget by .00027045 at strict above-support F1; the
adoption run stopped without running the other four objects, and the option
stays OFF. Sparse patch sampling, wider matching angles, existing normal
refinement, extra round-trip rejection and simple fusion-weight changes do not
provide a general repair. Browser execution of all 60 drill photos plus
original-RGB GLB export passes (63.7 s, 1.24 GiB WASM high water), but its mesh
is still open and about 31.6% of area untextured. 202 release library tests and
three geometry audit tests pass with Metal, along with strict clippy, threaded
WASM build, 18 Studio settings tests and Studio build. No new upstream algorithm
code/dependency or patent-clearance claim. See
[coherent-band evidence](../tests/evidence/coherent-band-review.json) and
[depth quality audit](DEPTH-QUALITY-AUDIT.md).

Independent drill stage controls now execute OpenCV SGBM, PyCOLMAP sparse SfM
and Open3D TSDF on own photo-derived evidence. A rotating-table feature region
lets COLMAP register 60/60, but native dense reconstruction still pits the drill.
SGBM worsens point accuracy on common pixels; Open3D fragments the current depths;
Poisson improves continuity but worsens mean geometry error. About 39.25% of
sampled native mesh surface points lack direct depth support within truncation,
with the audit's stated limits. Near-surface confidence and closed-completion
prototypes are rejected and reverted. The TSDF report now labels weight as
signed-distance vote mass, and original repeatable diagnostic runners include
analytic camera-Z/plane tests (16 photo-regression tests pass; strict release
clippy and format checks pass). No reconstruction defaults, app dependencies or
research gates change. The drill remains unresolved; see
[independent control evidence](../tests/evidence/drill-independent-controls.json)
and [depth quality audit](DEPTH-QUALITY-AUDIT.md).


Real-user capture camera work adds opt-in five-point initialization (attributed
BSD-3-Clause PoseLib adaptation), ambiguous P3P consensus, geometrically split
tracks and diverse seed selection. Existing algorithms/defaults are preserved.
138 photo release tests, strict library/test clippy and wasm32 release library
checks pass; browser execution is not established by this check. Nine real-batch
controls still register only 9–11/78 iPhone and 25–27/117 iPad views, with poor
inspected shape. These paths are experimental library APIs, not an app provider
or a successful pipeline repair. See [camera path evidence](../tests/evidence/camera-recovery-paths-review.json).

An unchunked CPU F32 MapAnything control removes the private MPS prototype's
constant-output symptom using the same F16-rounded checkpoint. Its complete
78-view prediction remains geometrically fragmented (median silhouette support
.650) and is rejected. The system Apple photogrammetry comparison also produces
no suitable object mesh. No printable STL has been accepted from either real
capture; fixed-camera Gaussian training cannot repair their cameras. See
[learned camera evidence](../tests/evidence/learned-camera-real-capture-review.json).


The separate Apache BootsTAPIR tracking control now has a reproducible MPS
sampling-parity workaround and bounded 78-view execution. Dense temporal anchors
plus wider localization tolerances register 39/78; combining independent own
SIFT/LightGlue landmarks registers 49/78. Neither is accepted: temporal-only
median cross-view silhouette support is .716, hybrid .903 (worst .812), and the
inspected point cloud remains poor/flattened. These use different point/view
sets, so support fractions are not paired accuracy scores. No learned tracker
is shipped in the app and no camera default changes. See
[point tracking evidence](../tests/evidence/point-tracking-camera-review.json).

The experimental general camera API also has an explicit pixel uncertainty
multiplier; default1 preserves prior gates. It changes correspondence tolerances
while retaining original photo coordinates and calibration. No automatic
uncertainty estimator is claimed. Final validation: 140 photo release tests,
strict library/test clippy and wasm32 release library checks pass.

A reusable object-only sparse-camera silhouette audit now reproduces these
checks from a COLMAP model and own masks, with dimension/convention validation
and bounded deterministic point sampling. Four analytic tests pass. An optional
caller-selected .95 median gate rejects the11-view SIFT,39-view temporal and
49-view hybrid controls. High support would still not prove accurate geometry.
See [runner instructions](../scripts/photo_regression/README.md).


Foreground ALIKED/LightGlue and native general recovery now register **76/78
iPhone** and **111/117 iPad** views without fixed angle steps. The reproducible
CPU runner uses pinned permissive code/weights; its iPad run matches 472 candidate
pairs (470 retained) without the earlier exhaustive SIFT overlap graph. Feature
extraction takes 143.9 s and matching 76.8 s; camera recovery takes 24.8 s. This
is a desktop research run above 96 photos, not a large-device benchmark or app
provider. Camera silhouette consistency still fails: median .859 iPhone / .729
iPad, on different landmark/view sets. Original-resolution SIFT refinement lowers
its own fitting residual to 1.182 px but does not repair the corrected iPhone
surfaces; both remain open with false grooves. Earlier surface trials made with
a private half-pixel input-conversion mistake are superseded. The production
scene writer was already correct. See
[learned-feature evidence](../tests/evidence/learned-feature-camera-review.json).

A concrete Gaussian/neural-surface loader failure is fixed: individually cropped
photos smaller than the shared training window are resampled from their observed
extent, with matching intrinsics. Existing integer-area sampling is preserved for
views that already fit; no unknown off-frame background is fabricated. Three
analytic crop/camera tests and all 268 release library tests pass. This removes an
execution blocker, not the need for accurate poses or a shape-quality result.

The bounded iPhone Gaussian control now finishes: 3,000 steps, 320-pixel crop,
192-cell mesh grid, 201 s training / 252 s including evaluation and meshing.
Its 91,384-triangle STL has one component and no boundary/nonmanifold edges.
Inspected grey renders reduce the stereo torso's false grooves, but lose real
relief and leg separation; it remains a coarse alternative. The 20.82 dB median
held-out in-mask PSNR measures appearance, not shape or camera accuracy. Poses
stay fixed. This is not the full 10,000-step fast preset or an equal-budget
comparison. The complete 111-view iPad native pipeline also finishes, but its
closed mesh lacks much of the body/legs (median mask IoU .349) and is rejected.
A stricter camera-tolerance control recovers only 11/117, so simply widening or
shrinking one global gate does not solve the batch. No geometry default changes.

The independent PyCOLMAP camera path now has a reproducible offline runner with
separate `fixed` / `focal` / `radial` switches. The iPad's same learned matches
recover 110/117 views and 6,988 points with focal/radial refinement. A public-runner
repeat reproduces the private control's lens and counts (42.0 s versus 31.6 s;
not a controlled timing comparison). Largest connected model selection avoids a
two-camera overfit with a deceptively low fitting error. Grey renders now retain
the torso and legs; arms and detail remain wrong. Raw sparse mask support rises
to median .900, still below the requested .95 check; differing points/views and
mask cleanup mean this is not a paired accuracy score. The normal import pipeline
and serial dense runs complete on all 110 recovered views.

A separate bounded learned-depth route uses the pinned Apache DA3-BASE CPU core,
physical virtual-camera rotations and four-view windows, anchored to observed own
SfM tracks. It completes 111 views in 91.4 s with 2.54 GiB peak RSS; the improved
110-camera control takes 86.4 s with 2.56 GiB. These are model-stage desktop
measurements, not a device/1536-photo benchmark. Standard staging filenames are
resolved explicitly through `photo-map.json`; depth planes are reframed to the
native working-crop intrinsics before fusion. The API wrapper and `evo` are not
used. External model/source are not bundled. The optional native port passes a
two-image CPU parity check, but its Metal intrinsics miss the predeclared .05 px
tolerance (.081 px); the public runner therefore stays on the official CPU core.

Matched grey renders show the tradeoff: learned depth reduces false grooves but
loses real hair and leg separation. Allowing 6 or 12 inconsistent silhouettes
instead of 2 recovers some missing arms/body width, while also distorting shape.
These remain explicit `hull_allowed` controls, not a default change. Even a closed
mesh with higher mask IoU is not an accepted faithful STL. Native stereo,
Gaussian/SDF and learned-depth paths remain available; camera/shape quality is
still unresolved on these real captures. All 24 relevant Python tests pass;
no Rust changed in this follow-up. Recipes and limits are in the
[runner README](../scripts/photo_regression/README.md); metadata-only results in
[windowed-depth evidence](../tests/evidence/windowed-learned-depth-review.json).

### Real-capture robustness follow-up, 2026-10-19

The experimental native general-camera path no longer silently skips bundle
adjustment above 128 recovered cameras. `--bundle-solver auto` retains the dense
solver for smaller fits and selects a matrix-free Schur/PCG solver above 128;
`dense`, `iterative` and `off` remain explicit comparison paths. A 180-camera
irregular, multi-height synthetic adjustment regression passes. On identical
initial real iPad tracks, both solvers agree numerically; complete incremental
recoveries can still diverge substantially. Neither test establishes accurate
recovery from 180 real photos. Intrinsics remain fixed in this native optimizer.

Native imported-camera/imported-mask staging now preserves upright JPEG bytes
and copies identity contrast, avoiding unnecessary full-resolution PNG expansion.
EXIF rotation and external-tool staging retain their existing handling. On the
117-photo iPad batch, all 332 prepared output files match the previous native
import byte for byte. This is an I/O fix, with no dense geometry default change.

Optional offline tools add SAM2.1 boundary refinement, conservative mask
preflight, cached foreground-correspondence filtering and coverage-checked view
selection. The Apache SAM source/checkpoint are content-pinned and external;
image inference excludes the installed combined-license progress dependency.
Preflight retains the coarse mask for unsupported new cuts or an exceeded native
cleanup budget. It changes 102 of 117 original masks and leaves 15 unchanged.
The normal native import passes its unchanged 2% cleanup gate.

Reviewed grey renders expose an important tradeoff: with the earlier 110-camera
model and the same selected 100 views, refined masks recover the visible leg gap
and a hand opening. False torso grooves and weak hair/arms remain. Allowing six
inconsistent silhouettes raises median mask IoU from .825 to .862 against all
110 refined input masks, but fills the leg gap again. Higher IoU is not faithful
geometry. Filtering matches recovers 116/117 cameras, yet its all-view and
selected-106 stereo meshes are worse visually; their all-116 mask IoUs are
.756 and .775. Keep these paths selectable, with no automatic promotion by count
or fitting error. The first iPad Gaussian attempt stopped at the disk reserve;
a guarded retry completed after space recovered. With the same selected 106
cameras, 3,000 steps / crop320 / mesh192 produce 99,556 triangles: less false torso
relief and fuller arms than stereo, but merged legs and lost real detail. Its
all-116-view mask IoU is .867 (stereo .775); held-out RGB in-mask PSNR is 22.78 dB.
These measure photo agreement and appearance, not shape accuracy. Cameras stay
fixed. Training takes327 s and evaluation/extraction another318 s on the shared
M1; budgets differ from stereo and this is not the full10,000-step fast preset.

All 272 native release library tests, the camera-export example test, strict
clippy, wasm32 library check and 53 photo-regression Python tests pass. These changes
have no new app provider, device build or browser execution validation. A faithful
detailed STL from either real-user batch remains unresolved. See
[recipes](../scripts/photo_regression/README.md) and
[metadata-only evidence](../tests/evidence/real-capture-robustness-review.json).
