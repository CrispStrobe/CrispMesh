# The `turntable` camera provider

`crispmesh-dense photos --cameras turntable`: cameras for one turn of ordered
turntable photos with a known lens, from this crate's own code
(`crates/dense/src/photos/turntable/`). No external program, no random choice,
plain Rust that builds for WebAssembly, so it is the camera provider for
phones and browsers as well as desktops.

It assumes what its name says: the photos are in capture order and each is the
previous one turned about one fixed axis (the object on a turntable under a
fixed camera). The axis should be roughly upright in the photos (within about
50 degrees) and the object roughly in front of the camera. A full turn is the
default; `--open-turn` is for photos that do not close one.

## Method

1. **Features** (`features.rs`): SIFT after Lowe, written here (the patent
   expired in 2020): Gaussian scale space of the doubled image, extrema of the
   differences of Gaussians located to sub-pixel accuracy, low contrast and
   edges rejected, orientations from gradient histograms, 4 x 4 x 8
   descriptors. On the contrast images, inside the object masks shrunk by two
   pixels, at most `--turntable-features` (6000) per photo.
2. **Matches** (`matching.rs`) between every photo and its next
   `--turntable-span` (4) photos, around the closed turn: nearest neighbour by
   descriptor distance in both directions, ratio test at 0.8, both agreeing.
   **Order check:** neighbours in name order must share at least 1.5 times as
   many matches (median over the turn) as the farthest pairs matched;
   otherwise the run is refused with a message to name the photos in capture
   order or to choose `--cameras colmap`. Measured: 2.9 to 14 for ordered sets
   (73, 37 and 25 photos), 0.93 to 1.08 for shuffled names.
3. **One motion** (`solver.rs`). The axis direction, the direction from the
   camera to the axis (its distance is set to 1, which fixes the scale) and
   the angle of every step are fitted to the matches of all consecutive pairs
   together, by their Sampson distances cut off at 3 px. The fit starts from
   the best of eleven axis tilts and both turning directions and refines the
   three shared numbers over shrinking ranges, re-fitting the step angles in
   every evaluation. A pair without image motion gets a step of zero (the
   Bunny's photos 13 and 14); a step far from the median is replaced by it.
4. **The turn closes.** Unless `--open-turn`, the steps, the one from the last
   photo back to the first included, are scaled to add up to 360 degrees.
   Steps that add up to less than 0.75 or more than 1.15 turns stop the run
   with that reason instead: the motion was not found. Correct solutions
   measured 0.78 (the Happy Buddha, whose rotation is poorly separated from
   translation; the adjustment then corrects the poses) to 1.07 (25 photos at
   15 degrees); a wrong axis gave 1.19 (a smooth bottle seen from a steep
   camera, `docs/OTHER-IMAGE-SETS.md`), and scaling it to one turn let it pass
   every later gate. The window was 0.6 to 1.4 for a while; all test sets
   have the same cameras under both.
5. **Tracks.** Matches that agree with these poses are joined; tracks of at
   least three photos are triangulated.
6. **Bundle adjustment** (`adjust.rs`) of all poses and points with the lens
   fixed: Levenberg-Marquardt, the points eliminated by a Schur complement,
   Huber loss at 1 px, the first camera held. Every pose has six free
   parameters; the turntable model was only the starting point. After the
   first adjustment the matches are checked again against the adjusted poses;
   then observations far off are dropped twice.

The Python prototype this was ported from
(`crates/dense/tools/turntable_prototype.py`) took step 3 from the essential
matrices of single pairs (OpenCV's five-point RANSAC) and medians over them.
An eight-point RANSAC written here could not do that: a small object in a
narrow field barely tells a 5 degree rotation from a translation, and its
axes scattered by 48 degrees. The joint fit needs no essential matrix and no
random sampling, and its raw step sums are closer to a full turn than the
prototype's (353 to 364 degrees against 352 to 375). The eight-point code is
still in `geometry.rs`, unused, for captures that are not turntables.

## Result

Same SAM masks for every provider, native dense stages, scanner F1 at 0.5 % of
the diagonal, `above_margin` (`all` in brackets). One run each for
`alicevision` and `colmap`, which are not repeatable.

| | Bunny | Armadillo | Dragon | Lucy |
| --- | --- | --- | --- | --- |
| `alicevision` | 0.963 (0.905) | 0.952 (0.923) | 0.834 (0.785) | 0.826 (0.790) |
| `colmap` | 0.965 (0.905) | 0.952 (0.924) | 0.844 (0.795) | 0.865 (0.828) |
| prototype (OpenCV SIFT, Python) | 0.967 (0.909) | 0.953 (0.926) | 0.840 (0.792) | 0.863 (0.827) |
| Rust solver on the prototype's matches | 0.968 (0.909) | 0.953 (0.925) | 0.837 (0.789) | 0.864 (0.827) |
| **`turntable`** (Rust features and solver) | **0.966 (0.908)** | **0.953 (0.926)** | **0.837 (0.791)** | **0.860 (0.824)** |
| `turntable` minus `colmap` | +0.002 | +0.001 | -0.007 | -0.005 |
| Registered | 73 of 73 | 73 of 73 | 73 of 73 | 73 of 73 |
| Features per photo, median: OpenCV / here | 1137 / 1107 | not recorded / 1266 | not recorded / 919 | not recorded / 378 |
| Matches over 292 pairs: OpenCV / here | 59 513 / 59 250 | not recorded / 73 072 | not recorded / 51 725 | not recorded / 30 521 |
| Points, observations: prototype | 6455, 32 422 | 7605, 40 147 | 4952, 25 761 | 2572, 13 766 |
| Points, observations: `turntable` | 6462, 32 531 | 7549, 39 973 | 4932, 25 914 | 2585, 13 690 |
| Reprojection median, p95 (px): prototype | 0.35, 1.19 | 0.36, 1.17 | 0.41, 1.31 | 0.44, 1.41 |
| Reprojection median, p95 (px): `turntable` | 0.35, 1.19 | 0.36, 1.16 | 0.41, 1.29 | 0.43, 1.42 |
| Camera recovery, seconds: `colmap` / prototype / `turntable` | 64 / 85 / 18 | 105 / 60 / 24 | 182 / 38 / 18 | 133 / 39 / 13 |

`turntable` times are features, matching and solving on an M1 with four
threads while another job used the machine (Bunny: 7.8 + 7.3 + 3.2 s).

- **The port.** With the prototype's own matches the Rust solver gives the
  prototype's cameras: rotations within 0.006 degrees and centres within
  0.009 % of the ring radius in the median on the Bunny, the Armadillo and the
  Dragon (largest 0.023 degrees), 0.10 degrees and 0.12 % on Lucy, where the
  different step fit shows most. Scanner F1 is within 0.001 on three objects
  and 0.003 lower on the Dragon.
- **The features.** With its own SIFT the provider is within 0.01 of COLMAP
  on all four objects, the criterion set for it. It is 0.007 and 0.005 below
  COLMAP on the Dragon and Lucy, the two objects with the fewest features,
  and, against its own solver fed with OpenCV's features, equal on the Dragon
  and 0.004 lower on Lucy.
- **Repeatability.** Two runs on the Bunny and on the Dragon, with two and
  with four threads, wrote identical `cameras.json` files, byte for byte.
  Nothing in the provider is random or depends on the order in which threads
  finish. (AliceVision's cameras moved by 0.7 degrees between runs.)
- **An open turn.** The first 50 Bunny photos (247 degrees) with
  `--open-turn --maximum-angular-gap-deg 360`: all registered, 0.33 px median.
  Against the full turn's cameras they differ by 0.29 degrees and 0.33 % of
  the radius in the median (1.07 degrees and 1.5 % at most): without the
  closure the ring is held less firmly. Without `--open-turn` the same photos
  are rejected by the closure gate.

## The closure gate

Step angles that are all a few percent too large or too small leave
reprojection and ring shape plausible. The first prototype passed every gate
on the Bunny with a 0.70 px median and scored 0.64 instead of 0.96; its orbit
swept 363.6 degrees from the first photo to the last. For a capture declared
a full turn (the default; `--open-turn` says otherwise), every camera provider
now has to close: what is left from the last photo to the first, per capture
step, must not be below minus a quarter of the median step and not above
`--maximum-closing-step-ratio` (2.5) times it. A unit test rebuilds that
363.6 degree ring and is rejected ("the cameras turn 363.6 degrees from the
first photo to the last, past a full turn"); the original files of that run
were overwritten, so the gate was not run on them.

## From photos to a scored STL in one command

`crispmesh-dense run --photos DIR --calibration scripts/turntable_mesh/calibrations/3dlf-pro.json
--output RUN`, masks and cameras left at their defaults: threshold masks, this
solver, the dense stages, nothing else started. 73 photos each, Apple M1
with 16 GB, one run per object; the scans are used for scoring only (F1 at
0.5 %, 1 % and 2 % of the scan's diagonal).

| Object | Whole run | Photos stage (cameras in it) | Stereo | Mesh | Check | F1, whole surface | F1, above the support |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Bunny | 93 s | 41 s (31 s) | 45 s | 5 s | 2 s | 0.904 / 0.939 / 0.956 | 0.964 / 0.995 / 1.000 |
| Armadillo | 83 s | 42 s (32 s) | 35 s | 4 s | 2 s | 0.924 / 0.972 / 0.987 | 0.952 / 0.997 / 1.000 |
| Dragon | 78 s | 33 s (23 s) | 39 s | 4 s | 2 s | 0.802 / 0.933 / 0.982 | 0.847 / 0.965 / 0.995 |
| Lucy | 50 s | 23 s (13 s) | 25 s | 1 s | 1 s | 0.822 / 0.940 / 0.965 | 0.861 / 0.985 / 0.999 |

Scores are from the default command at `ac36992` (threshold masks with the
contact shadow taken out, `--threshold-shadow 0.25`). Times are from the same
command with `--masks threshold` at `545356e`, before the shadow step, on an
otherwise idle machine; the runs at `ac36992` shared the machine with other
work and their times say nothing.

All 73 photos registered every time and every gate passed. Above the support
the scores equal those with SAM masks and the same cameras (0.966, 0.953,
0.837, 0.860 at 0.5 %). Sheets with the SAM-mask mesh beside
each: `checkpoint-renders/photos-to-stl-native-<object>.png`.

## Robustness

Default command (`threshold` masks, these cameras): the Bunny and Dragon
with 73 photos and the Thai statue at `ac36992`, the rest with the order check
and the closing window (which leave those three unchanged); 3DLF photos, scanner F1 at 0.5 / 1 / 2 % of the scan's
diagonal (evaluation only).

| Set | Photos (step) | Registered, gates | F1, whole surface | F1, above the support |
| --- | --- | --- | --- | --- |
| Bunny | 73 (5 deg) | 73, pass | 0.904 / 0.939 / 0.956 | 0.964 / 0.995 / 1.000 |
| Bunny, every 2nd | 37 (10 deg) | 37, pass | 0.902 / 0.938 / 0.956 | 0.961 / 0.995 / 1.000 |
| Bunny, every 3rd | 25 (15 deg) | 25, pass | 0.869 / 0.924 / 0.949 | 0.926 / 0.977 / 0.992 |
| Dragon | 73 (5 deg) | 73, pass | 0.802 / 0.933 / 0.982 | 0.847 / 0.965 / 0.995 |
| Dragon, every 2nd | 37 (10 deg) | 37, pass | 0.791 / 0.920 / 0.978 | 0.838 / 0.953 / 0.991 |
| Dragon, every 3rd | 25 (15 deg) | 25, **refused by the camera audit** (too few landmarks per view) | | |
| Thai statue | 73 (5 deg) | 73, pass | 0.872 / 0.933 / 0.953 | 0.935 / 0.995 / 1.000 |
| Happy Buddha | 73 (5 deg) | 73, pass (raw steps 0.78 turn, inside the closing window; refused by the closure gate before the window existed) | 0.779 / 0.912 / 0.967 | 0.817 / 0.944 / 0.985 |
| Asian dragon | 73 (5 deg) | 73, pass | mesh looks right; the scan evaluator cannot isolate the object from the platform in this scan | |
| Bunny, Dragon, names shuffled | 73 | refused by the order check | | |

Where it breaks: at 15 degree steps the Dragon keeps too few points per view
for the audit (the Bunny still passes, 0.03 lower above the support); the
Happy Buddha's face and the holes between its arms come out soft. Photos out
of order are refused, not sorted. YCB turntable sets are not supported: their
lens has tangential distortion, which the `radialk3` calibration cannot carry,
and their backdrop is dark.

## Limits and open points

- **Complete turns in order.** `--turntable-rings` needs every turn complete and
  with the same number of photos; `--turntable-turn-sizes 36,35,36` lists turns
  of unequal length (photos deleted after the capture). A turn's photos must not
  be shuffled with another's.
- **Turntables only.** A camera walking around an object at rest does not
  repeat one motion; that needs registration photo by photo, which is not
  written.
- **The default camera provider, on thin evidence.** For: equal
  scores, no external program, every platform, repeatable, three to ten times
  faster than COLMAP here. Against: it has been run on seven objects from one
  camera, lens and turntable, at 5 to 15 degree steps; the axis
  search assumes an upright axis and a centred object; it does not handle
  captures that are not turntables, which `colmap` does; the Dragon and Lucy
  are a little below COLMAP.
- **In a browser it is single-threaded.** The Bunny from its 73 photos
  completes in Chrome with WebGPU (351 s in all; features 57 s and matching
  55 s single-threaded; cameras equal to the native ones to 5e-11); see
  `crates/dense/README.md`.
- **The order check looks at medians.** A turn in name order with a few
  photos swapped passes the check and goes to the solver.
- **Memory and time grow with the mask's bounding box** (the doubled image
  and six blurred copies of it per octave): about 8 s for 73 photos of
  1749 x 1155 with four threads here. Matching compares all descriptors of a
  pair: 1 to 11 s for 292 pairs.
- Not tried: steps larger than 15 degrees, a strongly tilted axis.

## Large photos and several turns (2026-10-11)

**Working resolution.** `--turntable-working-size N` (default 2200): when the
longest photo side is larger, features are found and the cameras solved on
photos scaled to that side (area-filtered grey, nearest-neighbour masks), and
the lens and the observations are scaled back afterwards; the poses do not
change. The solver's bounds are in pixels (the 2 px support of the plane fit,
Sampson cut-offs, Huber at 1 px), so without this they tighten with every
megapixel: the YCB drill's DSLR photos (4272 x 2848) were refused with 14.6 %
plane support where the same photos at half size passed. With it they register
60 of 60 (median reprojection 0.67 px at full size), and the dense result equals
the earlier half-size-camera control. Photos at most 2200 px wide are not
touched: cameras and scene are byte-identical to the previous binary on a
2136 x 1424 capture. `0` keeps the old behaviour.

**Several turns.** `--turntable-rings N`: the photos are N complete turns of
equal length, one after the other, for example the same turntable seen by
cameras at several heights. The photo limit becomes 96 per turn.
`--turntable-turn-sizes 36,35,36` gives turns of unequal length instead (8 to
96 photos each, adding up to all photos; the number of turns follows); without
it every step below is unchanged. Across turns of unequal length a photo is
paired with the photo of the next turn at the same share of the turn, shifted
by the phase found.

1. Every turn is matched within itself as before. Across neighbouring turns only
   features that turn with the object are matched (inside the mask, or matched
   with motion within their own turn): cameras at different heights see the
   still surroundings alike at every angle, and those matches made the phase
   search and the links meaningless. The photo of the next turn at the same
   angle is found by probing two photos half a turn apart against all photos of
   the next turn; each photo is then matched with that one and its two
   neighbours.
2. Every turn is solved on its own. With `--turntable-region surface` a turn
   whose turntable plane cannot be fitted (seen edge-on from a level camera) is
   searched from its matches instead, with a warning.
3. A turn whose own cameras form a plausible ring (radius spread and distance
   from the plane within 5 %, measured like the ring gates) is placed in the
   first such turn's frame by a similarity, fitted with RANSAC to the points its
   solution shares with the turns already placed.
4. The photos of the other turns are registered one by one against the tracks
   of all photos registered so far, in rounds (so later photos also use earlier
   photos of their own turn): pose from the linked points, starting from the
   direct linear transform and from the poses of the registered cameras that
   share the most points (a turntable seen from above is a plane, where the
   linear transform fails), refined by Gauss-Newton with a shrinking inlier
   bound. A photo needs 20 points within 3 px.
5. A turn registered photo by photo rests on few points per photo, and a turn
   seen almost along the axis also drifts in a free adjustment: its photos
   barely tell its radius from its height. The turntable fixes what such a turn
   may be: one camera, turned about the axis the plausible turns share. The axis
   direction comes from the turns' rotations, a point on it exactly from pairs
   of photos of one turn (c_k - p = Q (c_j - p), Q the turn between them). The
   turn's photos are put onto that motion: the reference is the photo whose
   motion to the others is most nearly a turn about the axis, every photo keeps
   its angle about the axis (photos more than 3 degrees from such a turn take it
   from their neighbours), and the fixed camera's centre is the median of the
   centres turned back.
6. All matches, those across turns included, become tracks; one bundle
   adjustment, every pose free except those of the turns from step 5, which keep
   their turntable motion: their fixed camera (six parameters) and one angle per
   photo are adjusted with the rest, the axis held. Captures without such a turn
   use the plain adjustment unchanged.

A turn whose own solution fails (no points triangulated; on the mustard bottle
the camera almost straight down) no longer stops the run: it is warned about and
its photos are registered against the other turns. Default deadlines are
multiplied by the number of turns (given ones are kept): SAM on 300 photos of
4272 x 2848 needs more than the 30 minutes of one turn.

The ring gates apply to each turn separately, and so does the camera audit's
per-photo part (landmark support, reprojection): a photo with too little support
fails its own turn, not every turn. For the optical-axis gate the miss
is compared with the distance along the optical axis to the ring axis instead of
the ring radius, which is small for a camera looking down along the axis (one
turn only: unchanged). The SAM prompt box is the union of the coarse masks of
each turn, not of all photos.

On the YCB power drill's five DSLR cameras (60 photos each, 6 degree steps,
resampled into one common pinhole lens from each camera's own calibration; no
supplied poses, rig extrinsics or masks): turns 1 to 4 (elevations about 8, 36,
58 and 76 degrees) solved as clean rings and joined with 423, 347 and 247
agreeing points; turn 5 (almost straight down) solved as a distorted ring (radius
spread 15 %) and was registered photo by photo in two rounds (47, then 13
photos). 300 of 300 registered, median reprojection 0.73 px (p95 2.6 px) at
4272 x 2848, radius spreads 0.6 to 2.1 %, angular gaps 6.3 to 6.6 degrees.
Front end 8.5 min from cached matches (features and matching about 10 min more,
SAM about 30 min for 300 photos), peak memory 3.4 GB.

Synthetic tests: two turns 25 degrees apart joined to the truth (0.01 degree,
1e-3 relative translation) with one wrong match in ten, the same with one turn's
own solution deliberately broken (registered photo by photo), a resection with
one wrong match in four, and from points on one plane with a nearby start. Put back onto the turntable motion: a turn whose
true cameras are unchanged (below 1e-4 degree), and three photos 12 degrees off
returned to within 1 degree; the constrained adjustment returns a disturbed turn
to the truth.

With steps 5 and 6 the drill's five turns are unchanged in quality: F1 0.720 /
0.847 / 0.934 (photo-consistency fit; before 0.724 / 0.855 / 0.937), 0.798 at
0.5 % with mesh ICP (before 0.803). Camera distances to camera 3 agree with the
supplied rig (evaluation only) within 0.5 %, and every turn's depth bias is
within 0.05 %. On the YCB mustard bottle the cameras solve (300 of 300), but
the top-down turn keeps a +1.5 % depth bias (OTHER-IMAGE-SETS). One turn: the
same scene files, byte for byte, as before these steps (2136 x 1424 drill
capture).

## Experimental moving support region

`--turntable-region surface` extends feature detection one object bounding-box
side around its silhouette. Off-object matches with less than two pixels of
motion are discarded. A robust joint fit models a rotating planar support,
initializing the axis, direction to its centre and per-step angles before the
existing triangulation, free-pose bundle adjustment and camera gates. Object
points, selected by majority mask votes, locate the scene. No known board
coordinates or dimensions, reference geometry or supplied poses are inputs.

The initializer assumes a roughly uniform full turn, limited camera roll and
sufficient coplanar moving support features. Open turns are refused. Its
synthetic test covers a steep camera and wrong matches; the mustard experiment
registers 60 photos but still produces an unacceptable stereo mesh. This is an
opt-in experiment, not evidence of accurate mustard recovery. Ordinary object
feature detection and its axis search remain the defaults.

With `--keep-intermediates`, `sfm/photo-matches.json` retains the photo-derived
feature cache for repeatable diagnostics. Imported caches must come from the
same own photos and resolution; cache provenance is the caller's responsibility.
