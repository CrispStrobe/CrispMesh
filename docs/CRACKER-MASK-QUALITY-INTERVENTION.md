# Cracker box: one photo-only silhouette intervention

Status: **proposal and read-only inspection only**. No new masks, hull, mesh,
model inference, scanner access, or score were produced for this intervention.

## Why test masks next

The sealed 60-view hull scored fixed-gauge F@1% **16.33%**, versus **31.79%**
for rough OpenMVS, and had 92 nonmanifold edges. Its compact neutral preview
did not predict surface agreement. The input report explicitly calls the masks
"photo-derived coarse pose support, not a silhouette or ground truth". It lists
60 undistorted masks of 1297×1032 pixels, with foreground areas 35,251–67,183
pixels (2.63–5.02% of a frame). Only four source-mask sample angles had human
QA in the earlier stock experiment.

Read-only visual inspection of the undistorted RGB and masks at `NP3_000` and
`NP3_120` suggests different errors: the former mask extends past parts of the
box outline, while the latter has an irregular indentation that appears to
exclude box pixels. These are observations on two views, not a 60-view audit
or a measured attribution of the hull failure. The checkerboard, turntable,
hardware, shadow, and the box's contact edge are visible in the RGB. They give
specific photo-side negative and ambiguous regions to review.

## One intervention and its annotation contract

Replace **only** the 60 coarse hull-input masks with one independently
annotated set of object-only silhouettes. Annotate the exact 60 named
undistorted `dense/images/NP3_*.jpg` RGBs listed in the sealed
`turntable-fresh-openmvs-005/masks/report.json`, at their native 1297×1032
resolution. Use the RGBs alone while drawing: no old mask overlay, COLMAP
points/cameras, depth, scanner mesh/projection, supplied capture poses, hull,
or scored candidate. Do not infer hidden faces or regularize the outline to a
box. The foreground is the visible cardboard package, including printed
labels and flaps. Exclude the checkerboard, turntable, supports, cast shadows,
reflections, and all visible gaps. At the bottom contact, put the last visible
cardboard pixel inside and the support pixel outside. Where a 1–3 pixel
transition is genuinely ambiguous, trace the visual midpoint and mark the
uncertain arc for review. Do not silently expand the silhouette to preserve
features. Do not interpolate between neighboring views.

One primary annotator traces all 60. A second annotator independently traces
12 prespecified evenly spaced views (`NP3_000`, `030`, ..., `330`) from RGB
only. Compare the independent traces before showing either annotator the old
masks; adjudicate disagreements on RGB, and record the reason and affected
arc. A reviewer then inspects all 60 full-frame RGB/mask overlays and 4×
enlarged edge crops, with special attention to top corners, side edges,
bottom contact, checkerboard, and shadows. Any view with an unresolved
boundary ambiguity wider than three pixels or covering more than 5% of its
perimeter makes the **whole 60-view set abstain**. The masks are final only
after this review; no scanner-driven corrections or second candidate are
allowed.

Deliver exactly one 8-bit grayscale PNG per source image, named by the same
stem plus `.mask.png`, same width/height as its undistorted RGB, with labels
only 0 and 255. Require one connected foreground component, no foreground on
the image border, and positive area. Interior print colors remain foreground;
any hole needs explicit reviewer approval as a truly visible background gap.
Seal a manifest with the exact ordered image names, per-name RGB and mask
SHA-256, dimensions, foreground counts, annotation/adjudication status,
reviewer, and the annotation protocol/code version. Recheck all hashes before
and after candidate construction. Keep the old 60 masks immutable.

## Image-side measurements, before geometry

Compute and publish per-view values and their median, p95, and worst view.
The second annotator's 12 masks are an independent repeatability audit, not a
selection pool. The old-mask comparison is descriptive; it cannot be used to
choose among new candidates.

| Measure | Exact definition and interpretation |
|---|---|
| Old-mask excess | `|old ∖ reviewed| / |reviewed|`; coarse support that lies outside the reviewed object. |
| Old-mask omission | `|reviewed ∖ old| / |reviewed|`; visible object excluded by coarse support. |
| Old/new IoU | `|old ∩ reviewed| / |old ∪ reviewed|`, reported with the two directional errors so equal IoU cannot hide their different effects. |
| Boundary displacement | Bidirectional nearest-boundary distances in undistorted pixels: median and p95 for old versus reviewed, and independently for the 12 double annotations. Report top, side, and bottom-contact arcs separately. |
| Board/support confusion | On RGB, reviewer marks the visible checkerboard and any hardware/support patch touching or within 20 pixels of the box. Report old and reviewed foreground pixels inside those negative patches, and old/new excess within a 20-pixel exterior band along the bottom edge. The checkerboard/support labels never become hull masks. |
| Mask sanity | Foreground area and bounding box, connected-component count, holes, image-border hits, and maximum unresolved boundary width per view. |

The independently drawn 12-view masks should have p95 bidirectional boundary
displacement at most three pixels after adjudication; otherwise the annotation
process is not repeatable enough to release a hull candidate. Require zero
reviewed foreground pixels in marked checkerboard and hardware patches. A
nonzero count or failed format/hash check is a QA failure, not permission to
alter the cameras or threshold. Publish computed metrics when a validly formed
set abstains; a malformed file or changed hash fails closed before a full
report can be produced.

The read-only [mask QA command](../scripts/classical_backend/cracker_mask_qa.py)
implements exact 60/12 file inventories, sealed RGB/coarse-mask hashes,
native-size binary PNG validation, connected-component/border/hole checks,
old-mask excess/omission/IoU, exact Euclidean bidirectional boundary-distance
summaries, and the three-pixel repeatability gate. It rehashes all inputs
afterward and prints JSON to stdout; it does not generate masks or write a
report file. Run it only after photo annotation has been approved, with three
fresh/read-only input paths:

```sh
PYTHONDONTWRITEBYTECODE=1 .local-tools/colmap-sparse/venv/bin/python \
  -m scripts.classical_backend.cracker_mask_qa \
  --source /Volumes/backups/code/crispmesh-data/turntable-fresh-openmvs-005 \
  --reviewed /path/to/exact-60-reviewed-masks \
  --repeat /path/to/exact-12-independent-masks
```

Visible checkerboard/support patches, uncertain arcs, and edge crops require
the separate RGB reviewer record described above; the command cannot infer
those semantic labels from binary silhouettes. The tool has only been tested
on synthetic images and has not been run on the 60-view capture.

## Frozen geometry test and independent score

If the photo-only QA passes, perform **one** 60-mask substitution in the
existing visual-hull producer. Freeze all 60 named undistorted PINHOLE cameras
and poses, sparse-derived cube bounds, 160³ cell-center grid, `floor(u,v)`
sampling, all-visible conjunction, at-least-48-observing-views rule, mesh
export, resource limits, and abstention conditions from the [hull control
plan](CRACKER-VISUAL-HULL-CONTROL-PLAN.md). Do not rerun SfM or MVS. Seal the
new mask manifest, occupied-cell report, bare mesh hash, and neutral preview
before opening any reference. A boundary-touching or empty hull abstains.

Only then run the unchanged scanner-only scoring procedure with the frozen
shared 006-to-reference gauge, 2,048 area-weighted samples per direction,
seeds 2027/2028, and 0.5/1/2% thresholds. Reproduce the old rough/refined
and reference-self controls first. The predeclared success gate remains a rise
of at least five percentage points in **both** fixed-gauge precision and
recall at 1% versus rough OpenMVS, with no truncation and a plausible neutral
preview. Report F, both directional p95 distances, topology (including
nonmanifold edges), and the scanner-AABB-center diagnostic separately. Do not
use a fitted alignment to rescue a failed primary result, edit a mask after
seeing the score, or run alternate thresholds/candidates. This independent
scanner score is a single held-out outcome; the 60 RGB views themselves are
all used by the hull and are not a view holdout.

Interpretation is conditional. Large measured old-mask excess/omission plus
a successful replacement hull would implicate mask quality. A QA-passing
silhouette set that still scores poorly would shift attention to image-derived
camera error, gauge, voxel carving/topology, and true concavities; it would
not identify one cause by itself. A later **separate evaluation-only**
camera-oracle ablation could substitute supplied capture-rig poses while
holding this sealed mask set and grid fixed. Those poses must never guide mask
annotation or a product reconstruction algorithm, and that ablation must not
be mixed into this one-change experiment.

Manual PNG annotation and simple raster metrics need no model weights or
network service and can use cross-platform Python/Pillow/NumPy. If an
interactive segmenter is proposed later, record its exact code/checkpoint
license, hashes, platform/runtime requirements, and commercial-use terms
before adopting it; model output would still need the same blind visual QA.
