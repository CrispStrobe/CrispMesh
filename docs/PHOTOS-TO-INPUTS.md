# Photos to dense inputs

This step exists twice. `crispmesh-dense photos` (Rust, `crates/dense/src/photos/`)
is built around interchangeable providers, as planned in
[`ARCHITECTURE.md`](ARCHITECTURE.md), and needs no Python. The Python module
`scripts/turntable_mesh/photos_to_inputs.py` is its reference for everything
that is not a provider's own business (staging, coarse masks, hole cleanup,
contrast images, gates, sheets, events) and is described in the second half.

## The native step: `crispmesh-dense photos`

```sh
crispmesh-dense photos --photos data/bunny/rgb \
  --calibration scripts/turntable_mesh/calibrations/3dlf-pro.json \
  --output runs/bunny-front                     # masks: threshold, cameras: turntable, unless given
crispmesh-dense run --inputs runs/bunny-front/inputs --output runs/bunny-dense
crispmesh-dense photos --list-providers      # the table below, as JSON
```

With the default providers (`--masks threshold`, `--cameras turntable`) no
other program is started: photos and a lens calibration go in, masks, cameras
and undistorted photos come out, on any platform. `crispmesh-dense run --photos
DIR --calibration JSON --output RUN` does this step and the dense stages in
one command.

```
masks    stage photos, coarse masks -> MASK PROVIDER -> dark-hole cleanup -> masks/, contact sheet
cameras  contrast images -> CAMERA PROVIDER -> audit and ring gates
         -> undistortion of photos and masks, scene (inputs/) -> sparse overlay
```

Exit codes, gates, events (`masks` and `cameras` stages; `stage_started` now
also names the `provider`), the cancel file, the free-space floor and the
deadlines are those of the reference. Assumptions about the capture and the
calibration file are the same (sections below), except that the native step
also takes several turns (`--turntable-rings`, next section).

### Taking the photos

**How many.** One turn of 36 photos (10 degree steps) is enough for every method; more add little. On the
3DLF Bunny (F1 at 0.5 % above the support): 73 photos 0.964, 37 photos 0.961, 25 photos 0.926 (cameras still
found; the Dragon's 25 were refused for too few points per view).

**About 20 photos, with the neural surface.** At 19 photos (every 4th, about 19-degree steps) the depth maps
fail but the cameras are often still right, and the neural surface (`--surface fast`) does not need
neighbouring photos to match. Bunny, F1 at 0.5 / 1 / 2 % of the diagonal, whole object:

| Bunny | photos | F1 |
|---|---|---|
| ordinary mesh | 37 | 0.902 / 0.938 / 0.956 |
| neural surface, fast | 37 | 0.865 / 0.922 / 0.948 |
| ordinary mesh | 19 | 0.676 / 0.802 / 0.897 |
| neural surface, fast | 19 | **0.839 / 0.917 / 0.948** |

Armadillo at 19: ordinary mesh 0.696 / 0.851 / 0.946, neural surface 0.852 / 0.949 / 0.977 (73 photos,
ordinary mesh: 0.933 / 0.976 / 0.991). Rendered through the same cameras, the 19-photo surfaces show the
Bunny's ears, face, feet and tail and the Armadillo's shell pattern, hands and tail; the ordinary meshes are torn.
Neural surface times on the M1: about 30 min (Bunny), 20 min (Armadillo).

So below 25 photos per turn the landmark gate is lowered by default from 20 to 8 observations per photo
(`FEW_PHOTOS` in `photos/options.rs`; an explicit `--minimum-observations-per-view` always wins; 25 and more
photos are unchanged). At 19 photos the weakest correct views have only 9 to 12 observations. The ring checks
still refuse wrong cameras: of six 3DLF objects at 19 photos, Bunny, Armadillo and Thai statue pass; Dragon and
Lucy are refused (camera centres 7 to 8 % off the ring plane, gaps of 35 degrees), the Happy Buddha's turn is
not found (182 degrees); the Bunny at 15 photos is refused (9 % off the plane, radius spread 8 %). So about 20
photos work for objects with enough texture, 36 for all. More features do not rescue the failing cases: a four
times lower contrast threshold found four times the features on the 19 Bunny photos (4 495 against 1 118 per
photo) but only 1.8 times the matches; the descriptors do not recognise the points across that much rotation.

A warning while taking photos ("these photos are too far apart for this object") was tried and not shipped:
no cheap measure between two consecutive photos predicts which objects pass at 20 degrees
(`scripts/capture_spacing/`). Over all 73 pairs of each turn, SIFT inliers on the object at full size (median at
10 / 20 degrees): Bunny 12 / 10, Armadillo 39 / 17, Dragon 40 / 16, Lucy 30 / 17, Thai statue 59 / 20, Happy
Buddha 31 / 18; ORB inliers at 640 pixels rank them the same way (Bunny 68 / 32, Dragon 179 / 70). The Bunny has
the fewest matches and passes at 20 degrees; the Dragon, Lucy and Happy Buddha have more and fail. The solver's
own observation counts at 19 photos agree (Bunny 456 and Armadillo 499 pass, Dragon 415 and Lucy 547 are refused):
the failures are in the geometry of the whole turn, not a shortage of matches between neighbours. An apparent turn
from the matches' horizontal motion reads 3 to 10 degrees for a true 20 degree step. The photo count chosen in
*Take photos* (36 for every object, 20 with the neural surface) stays the guidance.

**With the app's camera.** *Take photos* (photos start, every platform: browser, Mac, iPhone, iPad) uses
the device's camera. The camera stands still on a stand, the object turns. With *Auto* on, a photo is
taken each time the picture has changed enough from the last photo (the object turned) and then stood
still for 0.6 s (`apps/studio/src/core/autoCapture.ts`: the share of pixels of 96-pixel grey copies that
changed by more than 12 grey levels; turned above 1.5 %, still below 0.3 %. On the 3DLF photos a 10 degree
step changes at least 2.9 % (Bunny) and 5.2 % (Dragon); strong noise and a brightness shift of 8 levels
change none, while the mean difference is diluted by the backdrop and missed every other step). Turning by
hand in steps works, and so does a stepping turntable. The photos (`capture_NNNN.jpg`,
full camera frames) become the run's photos, and since such a camera has no lens file the run uses the
camera's field of view where the app can ask the system for it (`fov:DEG`, below), else the automatic
lens (next paragraph). Checked end to end in headless Chrome with a video file as the camera
(`apps/studio/scripts/capture-check.mjs`; video of every 2nd Bunny photo, 37 positions at half size,
each held 2 s): Auto took one photo per position, 37 in 88 s; the run with the automatic lens registered
37 of 37 (0.38 px median) and closed the mesh in 34 s. The focal length found was 8 % short at that photo
size (1 073 against 1 164 px).

**Capture review and cropping (2026-10-18).** Capture accepts up to 1,536 photos total,
with up to 16 camera heights; each retained turn needs at least eight photos. Select
thumbnails to delete mistakes, then use the remaining photos in their original order.
Unequal turn sizes are passed to the engine, rather than assuming equal-length turns.
The 1–4× zoom is a digital crop, locked after the first photo because a capture shares
one lens. It preserves source pixel sampling and corrects the field of view.

Auto compares the frame actually encoded by the shutter, normalizes moderate exposure
changes symmetrically, checks drift throughout the 0.6-second settling interval and
waits at least one second between photos. Pausing and toggling Auto preserve its last
view. Tap the object in the live picture to select Auto's watched area; this helps when
moving shadows dominate the background. This is visual motion detection, not feedback
from a motor: it cannot guarantee a photograph at every programmed step.

On macOS 14 / iOS 17, optional foreground cropping previews the first, middle and last
photo with three controls: padding, search area and mask threshold. The foreground search
follows the previous object's box, then falls back to the full/central image if needed.
Reviewed sample masks are reused when saving. Photos without a valid foreground are
reported and omitted; remaining turns must still meet the minimum. Accepted JPEGs stay unchanged; omitted originals are saved in a separate
`-unmatched` folder. `--object-crop-padding 0.15` crops undistorted RGB, masks and texture
pixels together and shifts the principal point; its default `-1` keeps whole frames.
These controls do not establish camera accuracy or repair an inconsistent reconstruction.

**Without a lens file: `--calibration auto`** (turntable cameras only). The steps the solver finds from the
image motion scale with the focal length it is given, so a closed turn tells the focal length: the
solver starts from the photos' EXIF 35 mm equivalent focal length, else from `--lens-field-of-view`
(65 degrees across the longer side), corrects it by 360 degrees over the sum of the raw steps and solves
again until the sum is within 0.2 % of a turn (the matches are found once; on the Bunny 3 to 5 solves).
The principal point is the photo's centre and distortion is left out. Measured on the six 3DLF objects
(their factory lens has distortion; same matches, default dense stages, F1 at 0.5 / 1 / 2 % of the
scan's diagonal, whole object):

| Object | Focal found vs the lens file | Lens file | Automatic |
| --- | --- | --- | --- |
| Bunny | -0.9 % | 0.906 / 0.940 / 0.955 | 0.909 / 0.941 / 0.955 |
| Dragon | -3.3 % | 0.807 / 0.934 / 0.983 | 0.802 / 0.935 / 0.982 |
| Armadillo | -1.9 % | 0.933 / 0.976 / 0.991 | 0.938 / 0.978 / 0.992 |
| Lucy | +2.9 % | 0.847 / 0.944 / 0.968 | 0.844 / 0.943 / 0.968 |
| Thai statue | -0.8 % | 0.882 / 0.932 / 0.952 | 0.885 / 0.933 / 0.952 |
| Happy Buddha | +28 % | 0.793 / 0.908 / 0.972 | 0.783 / 0.903 / 0.969 |

The Happy Buddha is the case where it goes wrong: even with its lens file the raw steps add up to only 281
degrees (rotation and translation are hard to tell apart on it, TURNTABLE-SOLVER.md), so the focal length found
is 28 % too long; the mesh still comes out, 0.010 lower at 0.5 % (0.013 above the support). Nothing in the run
tells this case from a right one yet. Tried and withdrawn: measuring the turn after the adjustment of an open
turn instead of the raw steps (with the lens file it is closer to a turn: Bunny 360.9 against 355.4 degrees).
As a focal length signal it is worse: the adjustment takes up much of a wrong focal length in the geometry,
the turn hardly changes, and the search ended 9 to 13 % short on the Bunny, Dragon, Lucy and Armadillo (the
Happy Buddha +16 % instead of +28 %).

On the Bunny the start does not matter: 45, 65 and 90 degree first guesses (focal lengths 0.37 to 1.15
times the right one) all end at 2 306 to 2 308 px. Limits: the focal length is found to a few percent
(at the right focal length the raw steps add up to a little less than a turn: 355 degrees on the Bunny),
and it needs perspective: an object filling a good part of the picture (on a synthetic object a tenth of
the picture wide the steps hardly change with the focal length). A focal length outside 12 to 120
degrees of field of view is refused. Strong lens distortion (wide webcams) is not modelled; a lens file
is still better when there is one.

**When the camera's field of view is known: `--calibration fov:DEG`** (every camera provider). A pinhole
lens with DEG degrees of horizontal field of view across the photo's width (as the photos are given),
focal length `width / (2 tan(DEG/2))` in both directions, principal point in the centre, no distortion;
the lens is then fixed like a lens file's. Phones and some webcams report their field of view, which is
nominal: no distortion model, and a few percent off is possible. Measured on three 3DLF objects with the
true lens's field of view (41.17 degrees across 1749 px, i.e. its focal length 2328.3 px without its
distortion) and with 5 % less and more (default dense stages, F1 at 0.5 / 1 / 2 % of the scan's diagonal,
whole object; above the support in brackets at 0.5 %; scores in `tests/evidence/lens-field-of-view-review.json`):

| Object | Lens file | Automatic | `fov:` true (41.17) | `fov:` -5 % (39.11) | `fov:` +5 % (43.23) |
| --- | --- | --- | --- | --- | --- |
| Happy Buddha | 0.793 / 0.908 / 0.972 (0.834) | 0.783 / 0.903 / 0.969 (0.821) | 0.800 / 0.912 / 0.969 (0.841) | 0.799 / 0.914 / 0.971 (0.837) | 0.781 / 0.904 / 0.968 (0.819), found from the turn |
| Bunny | 0.908 / 0.941 / 0.956 (0.968) | 0.909 / 0.941 / 0.955 | 0.909 / 0.941 / 0.955 (0.969) | 0.908 / 0.939 / 0.954 (0.969) | 0.911 / 0.943 / 0.956 (0.970) |
| Dragon | 0.807 / 0.934 / 0.983 (0.852) | 0.802 / 0.935 / 0.982 | 0.803 / 0.934 / 0.982 (0.847) | 0.791 / 0.929 / 0.981 (0.841) | 0.802 / 0.935 / 0.982 (0.848) |
| Armadillo | 0.933 / 0.976 / 0.991 | 0.938 / 0.978 / 0.992 | 0.935 / 0.977 / 0.992 (0.957) | | 0.938 / 0.979 / 0.992 (0.960) |
| Lucy | 0.847 / 0.944 / 0.968 | 0.844 / 0.943 / 0.968 | 0.844 / 0.943 / 0.967 (0.879) | | 0.852 / 0.952 / 0.973 (0.880) |
| Thai statue | 0.882 / 0.932 / 0.952 | 0.885 / 0.933 / 0.952 | 0.885 / 0.933 / 0.953 (0.949) | | 0.885 / 0.933 / 0.953 (0.949) |

(Lens file and Automatic on the Bunny, Dragon, Armadillo, Lucy and Thai statue from the table above; the
last three with `fov:` were run later, true and 5 % wider only, no warnings, the turn closed; the Happy Buddha's two were rerun
with the same build and reproduce it.) A field of view within 5 % does as well as the lens file (within
0.012 at 0.5 %) and, on the Happy Buddha, better than Automatic (+0.016 at 0.5 % with the true and the 5 %
narrower one), because it does not depend on the turn adding up. The raw steps scale with the focal
length (Happy Buddha 281 / 297 degrees, Bunny 363 / 383 / 344, Dragon 373 / 393 / 353 for true / -5 % /
+5 %). On the Happy Buddha the 5 % wider field of view (a 5.2 % shorter focal length) brings them to 266
degrees, below the 270 degrees the solver accepts as one turn. Then, with a field of view only, the
turntable solver warns and finds the focal length from the turn instead, starting from the given one (the
row's result: the same +28 % focal length and scores as Automatic). A lens file is trusted: there the run
fails as before. Which direction a device errs in is not known; digital stabilisation or a cropped stream makes the
real field of view narrower than the reported one, i.e. the reported one too wide: the direction that
failed the Happy Buddha's turn and that the fallback covers.

In the apps, *Take photos* asks the system for the camera's field of view (`camera_field_of_view` in
`apps/studio/src-tauri/src/camera.rs`: AVFoundation's `videoFieldOfView` of the format whose size, or
else aspect ratio, matches the frames, for the back wide-angle camera on iOS and the default camera on
the Mac; across the frame's shorter side when the frames are portrait) and uses `fov:DEG` when it answers;
otherwise, in a browser, with the system camera or another camera chosen, it stays `auto`. The Mac this
was written on reports 0 ("unknown") for its built-in camera, so on the Mac this is usually `auto`; the
iOS answer is not yet checked on a device.

Measured on the YCB power drill (section "YCB power drill from the DSLR
cameras" in [`OTHER-IMAGE-SETS.md`](OTHER-IMAGE-SETS.md)), in order of effect:

1. **Pixels on the object.** The depth error of the matcher is at the limit
   the photos allow, so it shrinks with the pixels per millimetre on the
   object. The drill covering 2 % of a 1280 x 1024 photo (about 1.3 px/mm):
   F1 at 0.5 % 0.49; the same turn at 4.3 px/mm: 0.69, median depth error
   halved. Let the object fill most of the frame and keep the camera's full
   resolution. Large photos are fine: the cameras are solved on copies scaled
   to `--turntable-working-size` (2200 px) and the dense stage keeps only the
   region around the object once the photos exceed 2 GiB of grey.
2. **Several heights, rather than more photos per turn.** One turn leaves
   whatever its camera cannot see (undersides of overhangs, the inside of a
   handle). Five turns at elevations from about 8 to 85 degrees closed those
   openings (genus 26 to 18, F1 0.69 to 0.72). Photograph each height as a
   complete turn with the same number of photos (at least 8; 60 per turn
   worked), keep the turns one after the other in file order, keep the lens
   and focus, and pass `--turntable-rings N`. Turns of unequal length (photos
   deleted after the capture) are listed instead: `--turntable-turn-sizes
   36,35,36` gives the photos of each turn in file order (8 to 1,536 each, with at most 1,536 total, adding
   up to all photos) and sets the number of turns. A camera looking almost straight
   down is fine: its photos are registered one by one against the other turns
   when its own turn does not form a clean ring.
   This needs features: on the plain yellow mustard bottle the top-down turn
   stayed biased. With three or more heights reaching from near the table to
   steeply above, the dense stage treats the silhouette as the surface and
   lets depth only correct it (`silhouette_first`, crate README): smooth
   closed meshes on the mustard bottle and the drill, at about twice the
   time and with less fine relief. `silhouette_first=0` gives the earlier
   behaviour, which on the mustard bottle was worse than one side turn
   (OTHER-IMAGE-SETS).
3. **Something on the turntable that turns with the object** (printed
   pattern, newspaper, the marker mat) helps the camera solver with
   `--turntable-region surface`; the backdrop and room must not move.

The underside the object stands on is seen by no camera, whatever the heights;
two captures of differently placed objects are not combined.

### Mask providers

A mask provider turns the staged photos into one 0/255 mask per photo in the
photo's own frame. The dark-hole cleanup, the statistics and the contact sheet
follow for every provider.

| `--masks` | What it does | Needs | License | Desktop / phone / browser |
| --- | --- | --- | --- | --- |
| `threshold` (default) | grey threshold (`--threshold-level`: Otsu per photo by default, or a level), largest dark region inside `--threshold-envelope`, contact shadow taken out (`--threshold-shadow`) | nothing | this crate | yes / yes / yes |
| `import:DIR` | masks made elsewhere: 8-bit PNG, object above 127, named `capture_NNNN.png` in capture order or like the photo (`name.png`, `name.ext.png`) | nothing | this crate | yes / yes / yes |
| `external-sam` | SAM 2.1 prompted by the threshold masks (level 70), through `scripts/turntable_mesh/segment.py` | an interpreter with PyTorch (`--sam-python`), a SAM 2 checkout (`--sam-source`), a checkpoint (`--sam-checkpoint`), this repository (`--sam-repository`) | SAM 2 Apache-2.0, PyTorch BSD-3-Clause | yes / no / no |
| `sam` | the same prompts and choice as `external-sam`, in this process: SAM 2.1 Hiera-tiny as two ONNX graphs run by ONNX Runtime; SAM's single mask by default (`--sam-candidates several`: the best of its three proposals) | a build with the cargo feature `sam-onnx`; ONNX Runtime's shared library (`--sam-runtime`, or `ORT_DYLIB_PATH`); the model from `--sam-model DIR`, else fetched once (130 MB) from `huggingface.co/cstr/sam2.1-hiera-tiny-ONNX` into the cache and checked against pinned SHA-256 | model Apache-2.0, ONNX Runtime MIT, `ort` and `ureq` MIT OR Apache-2.0 | yes / no / no |

Which one to use was measured on the four test objects, from the photos in one
command with the turntable cameras, against SAM masks imported through
`import:DIR`, with the scanner evaluator (`crates/dense/README.md` has the
table). A plain threshold also takes the contact shadow on the turntable,
which ends as flat flakes around the feet. The provider therefore takes it
out: a mask pixel stays only when object core (darker than a quarter of the
way from the object's median grey to the level) lies at or below it in its
column. Lit upward faces have the object under them; shadow beside or in
front of the object has turntable under it. With that, the whole-surface
scores are within 0.005 of SAM on all four objects and above it on the
Dragon, and `threshold` is the default. `external-sam` remains for objects
that are not dark on a light backdrop.

With correct SAM masks (see below) the comparison stands. Scanner F1 at
0.5 %, whole surface / above the support, turntable cameras, native dense
stages:

| | threshold | SAM, MPS before the fix (old) | SAM, correct (`sam`, native) |
| --- | --- | --- | --- |
| Bunny | 0.905 / 0.967 | 0.908 / 0.966 | 0.912 / 0.966 |
| Armadillo | 0.913 / 0.950 | 0.926 / 0.953 | 0.921 / 0.947 |
| Dragon | 0.784 / 0.849 | 0.791 / 0.837 | 0.804 / 0.850 |
| Lucy | 0.819 / 0.862 | 0.824 / 0.860 | 0.809 / 0.848 |

SAM is within about 0.02 of the threshold masks either way: better over the
whole surface on three objects, mainly the Dragon, and 0.014 lower above the
support on Lucy, whose raised torch and forearm the SAM masks leave out on
some views. SAM is therefore an option for captures the threshold cannot
separate, not the default. Render sheets with one row per mask source:
`.local-tools/turntable-mesh/checkpoint-renders/sam-native-<object>.png`.

**SAM masks made with PyTorch on Apple MPS before 2026-10-06 came from a
partly wrong encoder** (PyTorch 2.7's `max_pool2d` on MPS is wrong for the
strided query view SAM 2's Hiera encoder pools). `segment.py` now pools a
contiguous copy; its MPS masks then equal the CPU masks and those of `sam`
(Bunny: 69 to 71 of 73 identical, lowest IoU 0.999996). Correct SAM masks hold
rows of small dark holes inside the object on some views (window-grid pattern,
up to about 2.5 % of the mask), which the dark-hole cleanup fills; its budget
is therefore 0.05 for `sam` and `external-sam` (and in `photos_to_inputs.py`)
unless `--hole-cleanup-budget` is given, and 0.02 for the other providers.

### Experimental background-colour masks

`--masks background` uses peripheral RGB/colour contrast for a centred object,
including light parts, and removes black padding connected to the photo frame.
It runs without a model or network on native and browser builds; threshold
remains the default. Inspect the contact sheet: similar foreground/background
colours, cast shadows and pale connectors can still fail. Marker-mat masking
with this experimental provider has not been validated.

```sh
crispmesh-dense run --photos PHOTOS --calibration LENS.json \
  --masks background --output NEW-RUN --texture
```

Texture is optional and does not change the source STL. See the
[image-set results](OTHER-IMAGE-SETS.md) for quality limits.

### Camera providers

A camera provider gets the contrast images (`work/contrast/capture_NNNN.png`:
gamma and CLAHE, `--contrast-gamma`, `--clahe-clip`, `--clahe-grid`), the masks
and the declared lens scaled to the photo size, and returns a solution: poses
of the registered photos, sparse points with their observations, and the lens
it used. It does not undistort anything.

| `--cameras` | What it does | Needs | License | Desktop / phone / browser |
| --- | --- | --- | --- | --- |
| `turntable` (default) | our own solver for one turn of ordered turntable photos ([`TURNTABLE-SOLVER.md`](TURNTABLE-SOLVER.md)): SIFT features in the masks, one rotation about a fixed axis fitted to all consecutive pairs, tracks, bundle adjustment with free poses and the lens fixed. Repeatable; `--open-turn` for photos that do not close a turn | nothing | this crate | yes / yes / yes |
| `markers` | a printed mat of ArUco markers under the object ([`MARKER-MAT.md`](MARKER-MAT.md)): markers detected in every photo, a pose per photo from all visible corners, the scene in the mat's millimetres and handedness. Verified on rendered photos only | the mat's description (`--markers-mat`, written by `crispmesh-dense mat`) | this crate; marker codes of OpenCV's `DICT_4X4_50` | yes / yes / yes |
| `colmap` | `feature_extractor` with one shared `FULL_OPENCV` camera fixed to the declared lens and the masks as COLMAP masks, `exhaustive_matcher` (or `sequential_matcher`, or `matches_importer` on a closed ring of pairs), `mapper` with focal length, principal point and distortion not refined | a COLMAP executable (`--colmap`, or `colmap` on the `PATH`) | BSD-3-Clause; dependencies carry their own | yes / no / no |
| `alicevision` | `cameraInit`, `featureExtraction` inside the masks, `imageMatching`, `featureMatching`, `globalSfM --lockAllIntrinsics true`: the commands of the reference | AliceVision executables (`--alicevision`) | MPL-2.0 (parts derived from libmv MIT); dependencies carry their own | yes / no / no |
| `import:PATH` | reads an existing solution: an AliceVision `.sfm` file, or a COLMAP model directory (`cameras`, `images`, `points3D` as `.bin` or `.txt`, also below `sparse/0`). Its photos may be named like the originals or `capture_NNNN.png` | nothing | this crate | yes / yes / yes |

AliceVision is given as an install prefix or as a wrapper:

- **Prefix** (`--alicevision /opt/alicevision`): `bin/aliceVision_TOOL` is
  started directly with `ALICEVISION_ROOT` set and `<prefix>/lib` in front of
  the library path (`DYLD_LIBRARY_PATH`, `LD_LIBRARY_PATH`; `PATH` on Windows).
  Further library directories go in `--alicevision-library-path` and further
  variables in `--alicevision-env NAME=VALUE`; a build against Homebrew
  libraries needs `--alicevision-library-path /opt/homebrew/lib`, for example.
  No interpreter is involved.
- **Wrapper** (`--alicevision /path/av`): called as `wrapper TOOL args`; a `.py`
  wrapper is run with `--python`.

COLMAP conventions that the provider converts: the principal point counts from
the image corner (ours from the centre of the top-left pixel, half a pixel
less; keypoints likewise); a pose is a world-to-camera quaternion `(w, x, y, z)`
and translation; the mapper refuses cameras with a distortion coefficient above
`Mapper.max_extra_param` (1 by default), so the provider raises that bound to
twice the largest declared coefficient (the 3DLF lens has k3 = -3.7; without
this only the initial pair registers). The option names differ between COLMAP 3 and 4
(`SiftExtraction.use_gpu` became `FeatureExtraction.use_gpu`); the provider
asks the executable for its version and uses the matching names
(`--colmap-cli auto`, the default; `3` or `4` force one).
`--random-seed` is not passed to COLMAP.

`colmap` is the first choice: on the four test objects it registered every
photo, was several times faster than AliceVision's global SfM and led to equal
or better reconstructions (`crates/dense/README.md`, "Camera providers through
the dense stages"). `alicevision` remains available and is the provider the
Python reference uses.

**Where the COLMAP executable has run.** In CI on Ubuntu (job "photos with the
COLMAP executable" in `.github/workflows/dense-native.yml`): the distribution's
COLMAP 3.9.1, version detected by the provider, on a synthetic textured capture
written by `crispmesh-dense synthetic --capture` (40 photos of a sphere on a
20 degree orbit): 40 of 40 photos registered, recovered elevation 19.99 degrees
against 20, largest step error 0.06 degrees, radius spread 0.06 %, and a dense
run on top that ends in a closed mesh of genus 0. On the development Mac there
is no `colmap` (Homebrew's bottle would upgrade Boost under the local
AliceVision build); the four real objects were run through
`crates/dense/tools/colmap_pycolmap.py`, a stand-in that accepts the provider's
command lines and runs the same COLMAP 3.11 library steps through the PyCOLMAP
wheel (`--colmap crates/dense/tools/colmap_pycolmap.py --python <python with pycolmap>`).
It has no `matches_importer`, so `--colmap-matching ring` is untested, as are
the COLMAP 4 option names (no COLMAP 4 executable has been run).

### After the provider

- **Closure gate.** The photos are taken as one closed turn unless `--open-turn`
  is given: what is left from the last photo back to the first, per capture
  step, must be a step like the others (not below minus a quarter of the
  median step, not above `--maximum-closing-step-ratio`, 2.5, times it). It
  applies to every camera provider and catches step angles that are all a
  little too large or too small, which no other gate sees.
- **Gates.** The audit works on the solution, whoever produced it: lens equal
  to the declared one (and reported as locked, where the program says), share
  of registered photos, observations in front of their cameras, reprojection
  per view, and the ring statistics. Limits and options are in the Gates
  section below.
- **Undistortion and scene.** Photos (the contrast images, as before) and masks
  are undistorted here with the map of OpenCV's `initUndistortRectifyMap`:
  photos by bilinear sampling of the 8-bit values, masks by nearest sampling.
  `prepareDenseScene` is no longer called. The scene is `inputs/` with
  `cameras.json`, `images/view_ID.png`, `masks/view_ID.png` and
  `sparse_points.npy`; image and mask paths are relative. Original capture RGB
  is undistorted with the same recovered lens into `texture_images/view_ID.png`;
  each camera row names it as `texture_image`. It is decoded with the same EXIF
  orientation handling as staging. Gamma/CLAHE images remain the matching inputs;
  texture export reads the unenhanced RGB. Originals are re-read in bounded batches
  after camera recovery, rather than keeping another staged set in memory.

### Output of the native step

As the reference, except: `inputs/images/` holds the undistorted photos and
there is no `native-prepared/`; `sfm/` holds `camera-audit.json`,
`ring-sanity.json`, `gates.json`, `expected-calibration.json` for every
provider, the AliceVision scenes for `alicevision` and `colmap/` (database,
`sparse/N`) for `colmap`; `frontend.json` has `providers` and
`provider_notes`; steps done in this process appear in `steps` with a
`(in process)` command and a one-line log.

### Options of the native step

`crispmesh-dense photos --help` lists them. Provider options are namespaced
(`--threshold-…`, `--sam-…`, `--alicevision-…`, `--colmap-…`); the names of the
reference command line remain as aliases (`--dark-threshold`, `--envelope`,
`--device`, `--describer-types`, `--describer-preset`, `--matching-method`,
`--sfm-option`, `--initial-field-of-view`, `--sensor-database`). Environment
variables: `CRISPMESH_MASKS`, `CRISPMESH_CAMERAS`, `CRISPMESH_ALICEVISION`,
`CRISPMESH_ALICEVISION_LIBRARY_PATH`, `CRISPMESH_ALICEVISION_ENV`,
`CRISPMESH_ALICEVISION_SENSOR_DB`, `CRISPMESH_COLMAP`, `CRISPMESH_PYTHON`,
`CRISPMESH_SAM_PYTHON`, `CRISPMESH_SAM_SOURCE`, `CRISPMESH_SAM_CHECKPOINT`,
`CRISPMESH_SAM_CONFIG`, `CRISPMESH_SAM_PYTHONPATH`, `CRISPMESH_REPOSITORY`.
Additions: `--stop-after masks`, `--list-providers`. Not offered: TIFF photos
(PNG and JPEG are read), `--alicevision-dense` and `--prepare-timeout`.

### Experimental general camera recovery (2026-10-18)

The Rust library experiment in `photos/incremental.rs` initializes calibrated two-view
poses, triangulates tracks, adds cameras through robust resection and adjusts their
poses freely. It takes no turntable axis, exact angles, orbit plane or capture order.
It is **not an available camera provider in the app or CLI**. Synthetic tests cover
shuffled viewpoints at different heights, duplicates, wrong correspondences, visible
cube faces and image noise; they check camera rotations and aligned 3D shape.

The default library path now retains consistent observations from contaminated tracks:
28 of 117 accepted iPad photos and 11 of 78 iPhone photos in the real-user controls.
`recover_with_resection(..., Resection::Linear, ...)` selects this existing path;
`Resection::P3p` selects calibrated minimal hypotheses adapted from the BSD-3-Clause
PoseLib solver. `recover(...)` keeps `Linear` as its default. No fixed step or known
turntable program is required by either path.

On the same own correspondences and lens, the P3P control recovers 27/117 iPad views
in 54.8 s and 11/78 iPhone views in 29.8 s, versus 28/117 in 143.0 s and 11/78 in
258.3 s for linear resection. These are single runs under different machine loads,
not general performance benchmarks. Both paths fail coverage and retain distorted
sparse geometry; neither produces an accepted STL. The existing turntable provider
continues to use its prior resection implementation. Source revision, hashes and
retained notices are in [camera-recovery-source-review.json](../tests/evidence/camera-recovery-source-review.json).
The external order-independent iPhone controls also remain fragmented. General handheld reconstruction and suitable
meshes from these batches remain open. The current dense bundle adjustment is capped
at 128 cameras in this experiment; larger reconstructions still need sparse or local
adjustment. No dense defaults changed. See
[real-user-capture-review.json](../tests/evidence/real-user-capture-review.json).

## The reference implementation (Python)

`python -m scripts.turntable_mesh.photos_to_inputs` takes a folder of turntable
photos and a lens file and produces everything the dense stage
(`dense_pipeline.py --inputs`) starts from: object masks, recovered cameras,
undistorted photos and the `inputs/` directory. It replaces the per-object
drivers that were written by hand for the Dragon, the Armadillo and the Bunny.

```sh
export CRISPMESH_ALICEVISION=/path/to/av.py            # or an install prefix
export CRISPMESH_ALICEVISION_DENSE=/path/to/av-dense.py
export CRISPMESH_SAM_PYTHON=/path/to/sam-venv/bin/python
export CRISPMESH_SAM_SOURCE=/path/to/sam2-checkout
export CRISPMESH_SAM_CHECKPOINT=/path/to/sam2.1_hiera_tiny.pt

python -m scripts.turntable_mesh.photos_to_inputs \
  --photos data/bunny/rgb \
  --calibration scripts/turntable_mesh/calibrations/3dlf-pro.json \
  --output runs/bunny-front
python -m scripts.turntable_mesh.dense_pipeline --inputs runs/bunny-front/inputs --output runs/bunny-dense
```

Exit code 0 means the cameras passed every gate and `inputs/` is ready. Exit
code 2 means cameras were recovered but rejected by a gate; nothing is handed
on. Exit code 1 means a step failed (tool error, deadline, disk, cancel).
`frontend.json` has the reasons in every case.

## What it assumes about the capture

- One turn of a turntable (or of the camera around the object), photographed
  by **one camera with a fixed lens and fixed focus**, 3 to 96 photos in the Python reference (native: up to 1,536 total), all the
  same size. The file order (natural order: `x_2` before `x_10`) is the capture
  order. (The native step also takes several turns within the same 1,536-photo total:
  `--turntable-rings` for turns of equal length, `--turntable-turn-sizes` for
  turns of unequal length.)
- A **dark object on a light backdrop**. The coarse masks are a grey-level
  threshold and the hole cleanup only fills dark holes; a light object on a
  dark backdrop is not supported. If threshold masks cover more than 90% of
  every photo, the run stops before camera recovery and keeps
  `mask-contact-sheet.png` and `masks-report.json` for inspection. Use a light
  backdrop, SAM, or your own masks. This conservative gate does not detect
  every bad mask and does not apply to the other mask providers.
- A **known lens**: a radial k1, k2, k3 model. The lens is locked during camera
  recovery; it is never refined. Earlier free-lens solutions on these sets
  folded the radial model while still showing low residuals.
- The object stays inside the frame and is the largest dark region in it (or
  inside `--envelope`).
- Nothing else is used: no scanner mesh, no supplied poses, no depth.

## What must be installed

Nothing is downloaded or installed by the command.

| Needed | Used for | License |
| --- | --- | --- |
| AliceVision build with `cameraInit`, `featureExtraction`, `imageMatching`, `featureMatching`, `globalSfM` and `prepareDenseScene` (see `docs/ALICEVISION-LOCAL-INSTALL.md`) | camera recovery, undistortion | MPL-2.0 (parts derived from libmv are MIT); its dependencies carry their own licenses |
| SAM 2 source checkout and a SAM 2.1 checkpoint (tested: `sam2.1_hiera_tiny.pt`) | segmentation | code and checkpoints Apache-2.0 |
| An interpreter with Torch and the SAM 2 dependencies (hydra-core, omegaconf with `antlr4`, `iopath`), plus NumPy, SciPy, Pillow | runs `scripts/turntable_mesh/segment.py` | |
| An interpreter with NumPy, SciPy, OpenCV and Pillow | every other step | |

AliceVision may be given as a wrapper script (called as `wrapper TOOL args`; a
`.py` wrapper is run with `--python`) or as an install prefix containing
`bin/aliceVision_TOOL`, in which case `ALICEVISION_ROOT` and the library path
are set for it. If the sparse tools and `prepareDenseScene` live in two builds,
give the second one with `--alicevision-dense`. If the SAM interpreter lacks
some pure-Python dependency, add directories with `--sam-pythonpath`.

## Calibration file

(Or `--calibration auto` for the turntable cameras, or `--calibration fov:DEG` for a known field of view: see "Without a lens file" above.)

```json
{
  "schema": "crispmesh_lens_calibration_v1",
  "model": "radialk3",
  "calibration_width": 583,
  "calibration_height": 385,
  "fx": 776.09, "fy": 776.62, "cx": 291.05, "cy": 184.91,
  "k1": -0.1718, "k2": 0.4123, "k3": -3.7170,
  "principal_point_convention": "pixel_centre",
  "sensor_width_mm": 36.0,
  "provenance": {"free": "form"}
}
```

- `fx`, `fy`, `cx`, `cy` are pixels **at the calibration resolution**. The
  centre of the top-left pixel is (0, 0) (the OpenCV convention).
- The distortion is `x_d = x (1 + k1 r^2 + k2 r^4 + k3 r^6)` on normalised
  coordinates. A Brown model with zero tangential terms is the same thing.
- If the photos have another size, the model is scaled: focal lengths by the
  size ratio, the principal point as `(c + 0.5) * s - 0.5`, coefficients
  unchanged. The photos must be a resize of the calibration frame; a different
  aspect ratio (a crop) is refused.
- `sensor_width_mm` is optional and nominal. It only fixes the unit of the
  focal length written into the AliceVision scene.
- `provenance` is not read. Say where the numbers come from.
- The raw 3DLF `calib_pro/intrinsics/rgb_optic.json` (`"model": "brown5"` with
  a nested `distortion`, `p1 = p2 = 0`) is accepted as it is.

`scripts/turntable_mesh/calibrations/3dlf-pro.json` is the 3DLF lens in this
format. It was stated for 583x385 while the photos are 1749x1155; scaling it by
three is an assumption that nobody has verified independently.

## Steps

| Stage | Step | Tool |
| --- | --- | --- |
| `masks` | copy photos as `capture_NNNN.png`, coarse dark-object masks | this module |
| | segmentation prompted by the coarse masks | `segment.py` (SAM 2.1) |
| | fill small dark holes | `silhouette_cleanup.py` |
| | masks, statistics, contact sheet | this module |
| `cameras` | gamma + CLAHE contrast images (features only) | this module |
| | list views, SIFT features inside the masks | `cameraInit`, `featureExtraction` |
| | apply the declared lens, locked | `cameraInit` |
| | pairs, matching | `imageMatching`, `featureMatching` |
| | cameras | `globalSfM --lockAllIntrinsics true` |
| | audit and ring gates | `alicevision_cameras.audit_scene`, this module |
| | undistorted photos | `prepareDenseScene` |
| | camera table and undistorted masks | `dense_all_views_inputs.py` |
| | sparse points drawn on photos | this module |

Every step is its own process group with a deadline; the free-space floor is
checked before each one. Creating a file named `cancel` next to the event log
stops the run. Photo copies, contrast images, features and matches are deleted
at the end unless `--keep-intermediates` is given. After that `sfm/final.sfm`
still names the deleted contrast images as its view paths; later stages read
`native-prepared/` and `inputs/` and only use the base names.

The undistorted photos in `native-prepared/` are the contrast images, as in the
hand-driven runs. The dense stage normalises grey levels inside the mask.

## Output

```
<output>/
  inputs/                cameras.json, masks/, sparse_points.npy
  masks/                 capture_NNNN.png, 0/255, original (distorted) frame
  sfm/final.sfm          cameras and sparse points
  sfm/camera-audit.json  reprojection per view, lens check
  sfm/ring-sanity.json   ring numbers
  sfm/gates.json         limits, numbers, pass/fail, reasons
  native-prepared/       <viewId>.png undistorted photos
  frontend.json          configuration, every command with exit code and time, gates, warnings, status
  frontend-config.json   the resolved options
  photo-map.json         capture name <- original file name, coarse mask numbers
  masks-report.json      mask area per photo, pixels SAM added to or dropped from the coarse region
  mask-contact-sheet.png sparse-overlay.png  events.jsonl  logs/NN-step.log
  work/                  coarse masks, SAM output, cleanup report (small)
```

## Events

With `--events PATH` the stage appends to that log (default
`<output>/events.jsonl`). It writes two stages, `masks` then `cameras`, each
with `stage_started`, `progress` and `stage_finished`. It does not write
`run_started` or `run_finished`; those belong to whatever drives the run.

| Event | Stage | Content |
| --- | --- | --- |
| `metric` | `masks` | `mask_area_median_pixels`, `mask_area_median_fraction` |
| `artifact` `mask_sheet` | `masks` | `mask-contact-sheet.png`: twelve photos, mask tinted green, dark pixels outside the mask in red |
| `metric` | `cameras` | `registered_views`, `input_photos`, `reprojection_median_pixels`, `reprojection_p95_pixels`, `ring_radius_spread_percent`, `ring_largest_gap_deg` |
| `artifact` `sparse_overlay` | `cameras` | `sparse-overlay.png`: sparse points on four undistorted photos with their masks |
| `artifact` `report` | `cameras` | `frontend.json` |
| `error` | either | a failed step, or `cameras rejected: <reasons>` |

`mask_sheet` and `sparse_overlay` are new artifact kinds and `masks` and
`cameras` are new stage names. They are not yet in `dense_events.ARTIFACT_KINDS`
/ `STAGES` or in `docs/ENGINE-CONTRACT.md`; the events are emitted with
`EventLog.emit` directly until they are.

## Gates

| Gate | Option | Default |
| --- | --- | --- |
| registered share of the photos | `--minimum-registered-fraction` | 0.8 |
| sparse observations in every registered view | `--minimum-observations-per-view` | 20 (8 below 25 photos per turn) |
| reprojection p95 in every view, pixels | `--maximum-view-reprojection-p95` | 4 |
| lens unchanged and still locked, radial model monotonic over the frame | (always) | |
| share of sparse observations in front of their camera | `--minimum-positive-depth-fraction` | 0.999 (1.0 would let a single stray point reject the cameras) |
| camera distance from the ring axis, max minus min, % of radius | `--maximum-radius-spread-percent` | 5 |
| camera distance from the ring plane, % of radius | `--maximum-out-of-plane-percent` | 5 |
| largest angle between neighbouring cameras | `--maximum-angular-gap-deg` | 30 (use 360 to accept a partial turn) |
| capture steps running against the turning direction | `--maximum-reversed-steps` | 0 |
| optical axis passes the ring axis within, % of radius, and points towards it | `--maximum-optical-axis-miss-percent` | 25 |

The Dragon, Armadillo and Bunny rigs measure about 0.5% radius spread, under 1%
out of plane and gaps of about 6 degrees, so the ring limits are loose: they
catch a broken solution, not a slightly inaccurate one. The gates certify that
the cameras are usable for photo reconstruction; they say nothing about the
accuracy of the shape.

## Options

Tool locations (flag, then environment variable): `--python` /
`CRISPMESH_PYTHON`, `--alicevision` / `CRISPMESH_ALICEVISION`,
`--alicevision-dense` / `CRISPMESH_ALICEVISION_DENSE`, `--sensor-database` /
`CRISPMESH_ALICEVISION_SENSOR_DB`, `--sam-python` / `CRISPMESH_SAM_PYTHON`,
`--sam-source` / `CRISPMESH_SAM_SOURCE`, `--sam-checkpoint` /
`CRISPMESH_SAM_CHECKPOINT`, `--sam-config` / `CRISPMESH_SAM_CONFIG`,
`--sam-pythonpath` / `CRISPMESH_SAM_PYTHONPATH`.

| Option | Default | Meaning |
| --- | --- | --- |
| `--device` | `mps` | SAM device. `segment.py` currently accepts `mps` and `cpu` only; `cuda` is refused until it does |
| `--threads` | 2 | CPU threads for every step |
| `--minimum-free-gib` | 10 | free disk required before every step |
| `--alicevision-memory-gib` | 4 | memory hint passed to the AliceVision tools |
| `--keep-intermediates` | off | keep photo copies, contrast images, features, matches |
| `--envelope` | `auto` | where the object can be: whole frame, or `x0,y0,x1,y1` in pixels (fractions if all are at most 1) |
| `--dark-threshold` | 70 | grey level below which a pixel counts as object, or `otsu` per photo |
| `--sam-multimask`, `--sam-preserve-holes`, `--sam-automatic-cues` | on | the `segment.py` flags; turn off with `--no-sam-...` |
| `--hole-cleanup-budget` | 0.02 | largest share of the foreground the dark-hole fill may add |
| `--contrast-gamma` | 0.5 | gamma of the feature images; 1 turns it off |
| `--clahe-clip`, `--clahe-grid` | 2.0, 8 | CLAHE on lightness; clip 0 turns it off |
| `--initial-field-of-view` | 45 | placeholder lens before the declared one is applied |
| `--describer-types`, `--describer-preset` | `sift`, `normal` | feature extraction |
| `--matching-method` | `Exhaustive` | `imageMatching` method |
| `--random-seed` | 0 | matching and SfM |
| `--sfm-option ARG` | none | extra token appended to `globalSfM`; repeat per token |
| `--sam-timeout`, `--features-timeout`, `--matching-timeout` | 1800 s | deadlines |
| `--sfm-timeout`, `--prepare-timeout`, `--small-step-timeout` | 900, 600, 600 s | deadlines |

The deadlines are sized for a 16 GB M1 that is also running a GPU job. On an
idle machine the 73-photo sets take about 1 min for SAM, 2 to 3 min for
features, 3 to 7 min for matching and 3 to 5 min for global SfM.

## Known failure modes

- **Camera recovery is not repeatable.** Three Bunny runs on identical photos
  and bit-identical masks gave three different camera sets: 8,463 to 9,087
  sparse points, per-view rotations differing by 0.3 to 0.9 degrees in the
  median (up to 1.8) after aligning the frames, and camera centres by 0.4 to
  2% of the ring radius. Running `globalSfM` alone a second time on
  byte-identical features and matches, same seed and thread count, moved the
  cameras just as much (median 0.7, up to 1.5 degrees), so the variation comes
  from the SfM step itself. The difference is a smooth once-per-turn drift in
  the viewing direction, not noise between neighbours (step sizes agree to
  about 0.03 degrees). Every solution has a 0.41 to 0.42 px median
  reprojection error, so the gates cannot tell them apart. Whether a single
  thread makes the step repeatable has not been tried.
- **One stray sparse point could reject good cameras.** In one of those three
  runs a single observation out of 42,382 lay behind its camera, and a limit of
  1.0 for `--minimum-positive-depth-fraction` (the policy the Dragon rig was
  accepted under) rejected the run. The default is therefore 0.999; pass 1.0
  for the strict policy.
- **Thin parts dropped by SAM.** The Armadillo's hands, the Bunny's far ear and
  the Dragon's horns are missing from the masks in runs of consecutive photos
  (up to about 7% of the figure). `frontend.json` warns when more than 3% of
  the coarse dark region is dropped, and the contact sheet shows it in red. The
  dense stage has a multi-view mask repair for this; the front stage does not
  merge the coarse mask back in, because the coarse mask also contains the
  contact shadow under the object.
- **Duplicate frames.** If the turntable did not move between two photos they
  get the same pose (Bunny photos 13 and 14). This is reported as a warning
  with the photo pairs; it is not a failure, but the dense stage then has a
  zero-baseline neighbour.
- **Uneven steps.** Recovered steps scatter by about half a degree around the
  nominal five on the 3DLF sets. Whether that is the turntable or pose error is
  not known; no gate tests uniformity.
- **Sense of rotation against the dataset's own poses.** On the Dragon, the
  Armadillo and the Bunny the recovered orbit turns the opposite way to the
  3DLF `poses_metric.json` (depth-derived) poses, with equal magnitudes. That
  points at a handedness difference between the photos and the dataset's depth
  and scan frame. Nothing here depends on those poses, but a comparison against
  the scanner mesh must allow for a mirrored frame.
- **Dark things other than the object.** With `--envelope auto` the largest
  dark region of the whole frame is taken as the object. A dark frame edge, a
  dark turntable or a deep shadow larger than the object will win; give an
  explicit `--envelope` or a lower `--dark-threshold`. A warning is recorded
  when the region touches the frame or the envelope.
- **More than 96 photos in the Python reference** are refused. The native engine accepts up to 1,536 photos total; this is an input bound, not a completed device benchmark.
- **No physical scale.** The reconstruction frame has an arbitrary scale and up
  direction.


## Experimental camera recovery choices (2026-10-18)

The Rust library’s `incremental::recover_with_options` keeps independent choices for
initial pose (`EightPoint` / BSD-adapted `FivePoint`), camera addition (`Linear`,
`P3p`, or one-to-one `P3pConsensus`), track retention (`Robust` / `Split`), and
starting-pair search (`Score` / `Diverse`, 1–256 attempts). Existing wrappers retain
eight-point, linear resection, strongest subtrack, and 32 score-ranked attempts.

`Split` can recover two real landmarks joined by one wrong correspondence; additional
models need three-view support and never reuse observations. Consensus registration
resolves competing feature/point associations with one vote per feature and world point,
and rejects coincident/collinear support. These are tested robustness mechanisms, not
a claim that the real captures are fixed. Five-point, consensus, diverse search and
track splitting still register only 9–11/78 iPhone and 25–27/117 iPad views in these
controls. All sparse sheets were inspected; coverage and geometry remain insufficient.
No option is advertised as a working general camera provider in the app.

Evidence: [camera-recovery-paths-review.json](../tests/evidence/camera-recovery-paths-review.json).

`measurement_error_scale` scales geometric correspondence tolerances (default1,
finite range0.125–16). It preserves original pixel coordinates and lens calibration;
use it for experiments whose feature localization uncertainty differs, such as
resized learned tracking. It is one global multiplier, not per-feature covariance
or automatic noise estimation. Wider gates can admit wrong geometry: the measured
coverage gain on real photos does not establish acceptable cameras.

A later foreground ALIKED/LightGlue experiment recovers **76/78 iPhone views**
with eight-point/P3P/diverse initialization and explicit uncertainty scale 4.
These are new correspondences; the earlier 9–11-view SIFT controls above remain
valid historical comparisons. Original-resolution SIFT refinement lowers its
own fitting residual, but the corrected untextured meshes still contain false
grooves and missing detail. No suitable STL or camera-default adoption follows.
Offline, hash-pinned learned matching, a switchable Rust recovery example and a
COLMAP-to-input adapter are now reproducible through the
[photo regression instructions](../scripts/photo_regression/README.md).
The input adapter's half-centred coordinate round trip is tested. An earlier
**private experimental adapter** incorrectly subtracted half a pixel; its old
surface trials are superseded. The production scene writer was already correct.
