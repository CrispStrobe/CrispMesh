# The marker mat

A printed sheet of square markers that lies under the object and turns with
it. With it, `crispmesh-dense photos --cameras markers` gets every camera pose
from the markers alone: no external program, the same result every time, the
scene in millimetres and with a known handedness.

**Status: verified on rendered photos only. No printed mat has been
photographed yet.** The repository's real board photos (YCB turntable sets)
show a plain chessboard, not markers. What a rendering does not contain is
listed under Limits.

## Print it

```sh
crispmesh-dense mat --size a4 --output mat/        # also: letter, a3
```

writes `crispmesh-marker-mat-a4.pdf`, `.svg`, `.png` (300 dpi) and `.json`.

| Size | Page (mm) | Markers | Marker side | Object zone |
| --- | --- | --- | --- | --- |
| `a4` | 210 x 297 | 28 | 24 mm | circle of 50 mm radius |
| `letter` | 215.9 x 279.4 | 30 | 24 mm | 50 mm |
| `a3` | 297 x 420 | 38 | 32 mm | 75 mm |

- Print the PDF at **100 %** ("actual size", no "fit to page"). Measure the
  scale bar in the middle of the sheet: 80 mm on A4 and Letter, 100 mm on A3.
  If it is off, the scale of the result is off by the same factor.
- Matt paper, flat: glue or tape it to something rigid. A curled sheet is not
  the plane the solver assumes.
- Keep the white space around the markers white. Do not trim the sheet into
  the markers.

## Capture

- Put the mat on the turntable and the object in the middle, inside the dashed
  circle. The object may cover the scale bar and the cross. It should not cover
  markers it does not have to; whatever it hides is simply not used.
- The mat must turn with the object and must not slip under it. A camera
  walking around a mat at rest works the same way.
- Every photo needs at least five whole markers (`--markers-minimum`); more
  are better, and markers on opposite sides of the object fix the pose best.
- Use the lens calibration of the camera as for every other provider. The lens
  is not estimated from the mat.

```sh
crispmesh-dense photos --photos DIR --calibration lens.json --output RUN/frontend \
    --cameras markers --markers-mat mat/crispmesh-marker-mat-a4.json --masks ...
```

## What the provider does

1. **Detection** in every photo, in the photo's own (distorted) frame: regions
   darker than their surroundings, a quadrilateral per region, its four sides
   located to sub-pixel accuracy from the grey gradient and fitted as straight
   lines after removing the lens distortion, corners where the lines meet, the
   4 x 4 code read through the quadrilateral and matched against the mat with
   one cell of error correction.
2. **Pose per photo** from all visible corners: a plane-to-image homography,
   then a robust refinement of the reprojection error with the lens fixed.
   The two poses a distant plane allows (tilted towards or away from the
   camera) are both refined; the better one is kept, and where they are close
   the camera height that the confident photos agree on decides. Corners more
   than 3 px from their reprojection are dropped.
3. **All photos together** give the one thing they share: whether the printer
   scaled the page's height differently from its width. The estimate is
   reported; `--markers-aspect auto` applies it, `--markers-aspect 1.004` sets
   a measured value. Nothing else is shared: with the mat's geometry and the
   lens given, the best pose of each photo does not depend on the other
   photos, so a joint adjustment of all poses would return the same poses.
4. **The scene** is in the mat's frame: origin at the page centre, +X to the
   right and +Y to the top of the printed page, +Z up out of the page,
   right-handed, millimetres. `cameras.json` gets
   `"scale": {"unit": "mm", "source": "markers"}`. The marker corners are the
   landmarks the gates audit; `sparse_points.npy` holds points of the space
   above the mat that the masks allow, because the dense stages take the
   object's place from that file and the markers lie around the object.

Gates: the shared ones (`docs/PHOTOS-TO-INPUTS.md`: share of registered photos,
at least 20 corner observations per photo, reprojection, ring sanity), plus
per photo at least `--markers-minimum` markers and at most 2 px RMS. A photo
that fails gets no pose and a reason in `frontend.json` and
`sfm/markers/report.json`; `sfm/markers/detections.json` lists every marker
found.

## What it guarantees

- **Scale.** One scene unit is one millimetre of the print. It is as right as
  the print is: check the scale bar.
- **Handedness.** The markers carry ids at known places and their codes do not
  read mirrored, so the frame cannot come out as its mirror image.
- **Repeatability.** No random numbers and no dependence between photos: the
  same photos give the same poses.
- **Every platform.** Plain Rust in this crate; it builds for WebAssembly.

## Measured (rendered photos)

Renderings by `crispmesh-dense mat --capture`: the A4 mat under a dark object
(a 64 mm wide, 90 mm tall capsule), turning under a fixed camera 420 mm away,
1200 x 900 pixels, a lens with radial distortion, defocus blur of 0.8 px,
sensor noise of 2 grey levels, contact shadow. Poses are compared with the
exact ones directly, without aligning the two sets.

| Capture | Registered | Markers per photo (min / median) | Rotation error, median / max | Position error, median / max |
| --- | --- | --- | --- | --- |
| Base, 25 degrees elevation, 36 photos | 36 of 36 | 20 / 22.5 | 0.0008 / 0.004 deg | 0.006 / 0.03 mm |
| Elevation 5 degrees | 24 of 24 | 6 / 16 | 0.009 / 0.02 deg | 0.06 / 0.21 mm |
| Elevation 8, 10, 15 degrees | all | 15 or more | at most 0.004 / 0.019 deg | at most 0.03 / 0.12 mm |
| Elevation 20 to 60 degrees | all | 17 or more | about 0.001 / 0.01 deg | about 0.01 / 0.07 mm |
| Blur 1.5, 2.5, 4 px | all | 20, 20, 15 | 0.002, 0.009, 0.04 deg median; 0.09 max at 4 px | 0.02, 0.07, 0.25 mm median |
| Noise 6, 12 grey levels | all | 20 | 0.002, 0.003 / 0.008 deg | 0.01, 0.02 mm median |
| Exposure x0.35, x0.6, x1.25 (paper clipped) | all | 20 | at most 0.003 / 0.004 deg | at most 0.03 mm median |
| Object 96 mm wide, 170 mm tall; also at 12 degrees | all | 11; 16 | 0.001, 0.002 / 0.005 deg | 0.01 mm median |
| 640 x 480 pixels | all | 20 | 0.002 / 0.02 deg | 0.02 / 0.15 mm |
| Camera 800 mm away | all | 21 | 0.003 / 0.007 deg | 0.04 / 0.09 mm |
| 12 degrees, blur 2 px, noise 6, exposure x0.6 | all | 15 | 0.018 / 0.04 deg | 0.14 / 0.24 mm |
| A3 mat, camera 600 mm away | all | 25 | 0.001 / 0.009 deg | 0.015 / 0.09 mm |

The scale of the recovered orbit was within 0.03 % of the truth in every row
(within 0.001 % in most).

- **Against OpenCV.** On the 36 base photos this detector found 808 markers,
  OpenCV 4.10's ArUco detector 800, all of them among the 808. Corner error
  against the truth: median 0.035 px here (95 % below 0.15 px); OpenCV 0.71 px
  with its default settings and 0.25 px with sub-pixel refinement.
- **Through the dense stages.** The base capture with `--cameras markers` and
  the rendered object masks, then `crispmesh-dense run`: a closed mesh whose
  side has a radius of 31.97 mm (true 32.00, -0.08 %) and whose top is at
  90.00 mm (true 90.00). Camera recovery took one second for 36 photos.
- **Print aspect.** A mat description stretched by 1 % against the rendered
  print is estimated as 1.010 from five photos (unit test).

## Limits

- **Not photographed.** All numbers above are from renderings of an ideal
  print: perfectly flat, matt, evenly lit, and a lens that is exactly the
  declared model. Paper curl, glossy toner, uneven light, motion blur and
  lens calibration error will add to them. A real capture is the next test.
- **Masks.** The markers are dark, like the object. With `--masks threshold`
  the provider's own detection and pose are therefore used once more during
  the masks stage. Every marker is compared with the photo cell by cell, and
  the cells that look as printed (with all their neighbours) are taken out of
  the dark region before the object is picked; cells the object covers stay,
  so the object is not cut, and a marker half hidden by the object keeps a few
  cells at the object's outline. The grey level is Otsu's level of the pixels
  that look at the mat's object zone, where there is only paper and object, so
  the surface the mat lies on does not decide it. No option is needed.
  Rendered captures against the rendered masks:

  | Surface around the mat | Level used | Mask IoU, median (min) | Extra pixels, median (max) | Missing, median (max) | Object from the dense stages |
  | --- | --- | --- | --- | --- | --- |
  | white (0.9) | 109 to 110 | 0.985 (0.980) | 1.3 % (1.7 %) | 0.2 % (0.8 %) | radius -0.08 %, height -0.01 % |
  | grey (0.66) | 109 to 110 | 0.986 (0.981) | 1.3 % (1.7 %) | 0.1 % (0.7 %) | radius -0.08 %, height -0.03 % |
  | dark (0.15) | none | refused | | | |

  A surface as dark as the object cannot be separated from it by a threshold
  wherever the object is seen against it. This is refused with a message that
  says so ("the surface around the mat is as dark as the object ...") instead
  of taking the table for the object; use a light surface, `external-sam` or
  `import` there. Without the marker step the same level gives 2.9 % extra
  pixels in the median and 5.9 % at most.
- **The lens is taken as given.** A wrong calibration tilts and shifts the
  poses and does not show as a failed gate until it is gross.
- **A marker that is partly hidden is not used**, and nothing outside the mat
  is: an object that covers most of the sheet from a low camera leaves few
  markers. Use the larger mat.
- **Flat mat only**, and one mat. The description format allows any marker
  list in the plane z = 0 (and other square dictionaries with a one-cell
  border), so other layouts can be described by hand.
- The dense stages do not read `scale` yet; it is in the scene for whatever
  writes the mesh out with units.

## The first real print: a checklist

No printed mat has been photographed yet, so the first capture is a test of
this provider as much as a scan.

1. Print `crispmesh-marker-mat-a4.pdf` at 100 %. Measure the bar: 80 mm. Note
   the measured length, and the page's long edge between the outer marker
   edges (262 mm on A4) if a ruler allows.
2. Tape the sheet flat on the turntable, centre cross on the axis as well as
   the eye can tell. A light surface around it.
3. First **without an object**: 24 or more photos of one turn from the height
   the scan will use, the whole sheet in view. Then **with the object** in
   the circle, the same way.
4. Run both sets with `--cameras markers --markers-mat ...json --masks
   threshold` and the camera's lens file.

What should come out, from the renderings: every photo registered;
`provider_notes.markers` in `frontend.json` with 15 or more markers per photo
without an object and `rms_pixels_median` well below 1 (0.06 in the base
rendering; a real print and lens will be higher, and up to about 0.5 would not
worry); `print_aspect.estimate` within 0.3 % of 1; ring radius spread and
out-of-plane both below 1 %; steps as even as the turntable.

What the gates say when it fails:

| Message | Likely cause |
| --- | --- |
| `no pose from the mat for N photos: ... (K markers decoded; 5 needed)` | markers too small in the photo, out of focus, glare on the toner, or hidden; photograph from higher or closer, or use the larger mat |
| `... marker corners reproject with X px RMS; limit 2` | the sheet is not flat, or the lens file does not belong to this camera or zoom |
| `the photos fit a print whose height is X % off against its width` (warning) | the printer scaled one axis; pass `--markers-aspect auto` or the measured ratio |
| `per-view reprojection exceeds declared bound`, `ring radius spread ...` | as above: flatness or lens; a mat that slipped on the turntable shows as uneven steps |
| `the surface around the mat is as dark as the object ...` | see Masks under Limits |
| `the turn does not close ...` | the photos are not one full turn: `--open-turn` |

If it fails or looks wrong, send back: the photos of the empty mat (they are
enough to find the cause), the lens file, the measured bar length,
`frontend.json`, and `sfm/markers/report.json` and `detections.json` from the
run's output.

## The description file

`crispmesh_marker_mat_v1`, millimetres:

```json
{
  "schema": "crispmesh_marker_mat_v1", "name": "a4", "unit": "mm",
  "page": {"width": 210.0, "height": 297.0},
  "dictionary": {"name": "DICT_4X4_50", "bits": 4, "border_cells": 1},
  "marker_size": 24.0, "object_radius": 50.0,
  "scale_bar": {"from": [-40.0, -16.67], "to": [40.0, -16.67], "length": 80.0},
  "markers": [
    {"id": 0, "code": "b532", "corners": [[-80.0, 131.0, 0.0], [-56.0, 131.0, 0.0], [-56.0, 107.0, 0.0], [-80.0, 107.0, 0.0]]}
  ]
}
```

`corners` are top-left, top-right, bottom-right, bottom-left as printed.
`code` is the marker's cells row by row from the top-left, 1 = white, as
hexadecimal. The markers are those of OpenCV's ArUco dictionary `DICT_4X4_50`
(ids 0 upwards), so other ArUco software reads the mat too; the 50 codes were
read from OpenCV 4.10 and are in `crates/dense/src/photos/markers/mat.rs`.
