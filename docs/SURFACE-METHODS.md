# Surface methods: what we measured and where we go

State on 2026-10-13. This page records one round of comparisons on two captures and the plan that
follows from it. Scans are used for scoring only; every mesh has its own similarity fit to the scan,
so differences of a few hundredths in F1 at 0.5 % are within the fit. Numbers are F1 at 0.5 / 1 / 2 %
of the scan's diagonal and the mean of accuracy and completeness as a fraction of the diagonal.

Captures: YCB power drill and mustard bottle, 300 DSLR photos each from five camera heights, our own
masks and our own recovered cameras (see [OTHER-IMAGE-SETS.md](OTHER-IMAGE-SETS.md)).

## Our own variants, like for like

Same masks (grown, see `mask_grow`), same silhouette hull, same cameras:

| | Drill | mean | Mustard | mean |
| --- | --- | --- | --- | --- |
| Silhouette hull only | 0.697 / 0.871 / 0.991 | 0.0046 | 0.794 / 0.910 / 0.999 | 0.0037 |
| Hull corrected by fused depth (`silhouette_first`, the default for several heights) | 0.818 / 0.921 / 0.998 | 0.0033 | 0.766 / 0.918 / 0.997 | 0.0037 |
| Hull refined against the photos, no depth maps (prototype) | 0.860 / 0.965 / 0.996 | 0.0030 | 0.752 / 0.909 / 0.999 | 0.0038 |

Reading:

- On the textured drill, depth evidence is worth 0.12 in F1 at 0.5 %; refining the hull directly
  against the photos is worth more, and looks as smooth as the hull.
- On the plain mustard bottle nothing beats the silhouette: every photo-based correction is equal
  within the fit or slightly worse. A single-colour surface gives the photos nothing to agree on.
- The silhouette alone needs no matching at all (hull and mask repair about 75 s, mesh 2 s on 300
  DSLR photos), against 8.5 to 10.5 minutes with depth. A mode that stops there does not exist yet.

### The photometric refinement prototype

Not in the pipeline; a NumPy script outside the repository. Anchor vertices of the hull mesh search
along their normal for the offset at which the photos agree: a 5 x 5 patch on the mesh's tangent
plane is projected into the most frontal view that sees the vertex and into up to five views 8 to
35 degrees away; the score is the mean normalised cross-correlation with the reference patch.
Visibility comes from the mesh. Accepted offsets are spread over the mesh as a smooth field (weighted
diffusion), the mesh moves inward only, and the search repeats with a smaller range (6, 3, 1.5
voxels). About 100 s for the three rounds on 300 half-size photos.

Acceptance decides everything. Without any test beyond a score threshold the drill reaches
0.842 / 0.968 / 0.999 and the mustard drops to 0.707 / 0.896 / 0.998 (over-carved sides, broken cap).

| Acceptance | Drill | Mustard |
| --- | --- | --- |
| score threshold only | 0.842 / 0.968 / 0.999 | 0.707 / 0.896 / 0.998 |
| best score at least 0.15 above the median over the offsets (a distinct peak) | 0.860 / 0.965 / 0.996 | 0.752 / 0.909 / 0.999 |
| patches with a grey-level deviation of at least 4 | 0.811 / 0.966 / 0.999 | 0.730 / 0.921 / 0.991 |
| field less one local scatter of the moves | 0.771 / 0.924 / 0.994 | 0.775 / 0.919 / 1.000 |
| all three | 0.716 / 0.880 / 0.991 | 0.798 / 0.913 / 1.000 |

The strict end is safe on the plain object and gives the textured one's gain away; the distinct-peak
test keeps the gain at a small cost on the plain object. Points in between (a peak 0.25 above the median: drill
0.784 / 0.956 / 0.996; the peak test with half the scatter gate: 0.762 / 0.926 / 0.994) lose the drill's gain
without helping the mustard, whose variants all lie within the fit of each other. Rendered, the
distinct-peak result is clean and slimmer than the hull on the drill and is the hull with two or three
small dents on the mustard.

## Other methods on the same captures

### Gaussian splatting (Brush)

[Brush](https://github.com/ArthurBrussee/brush) (Apache-2.0, Rust and wgpu, commit `1388f74`) run on
its own with default settings on the drill: our cameras and masks in its input format, original colour
photos at half size, 30 000 steps, about an hour on an Apple M1.

- Held-out photos are rendered sharply (30 views, median 25.7 dB inside the mask). That also confirms
  the recovered cameras.
- Its surface is less accurate than its pictures. Points where its rendering turns opaque along each
  ray: 0.685 / 0.846 / 0.946, mean 0.0058. A mesh from those depths (plain voxel fusion, marching
  cubes, no smoothing, our conversion): 0.355 / 0.579 / 0.831, grainy and somewhat inflated, yet with
  relief our meshes lack (chuck rings, housing seams).
- Splat centres lie about 1 cm inside the object, as expected of opaque objects in this method.

Plain splatting optimises appearance; nothing in it asks for one continuous surface.

### Reference runs on a GPU

Two published surface methods on both captures (half-size crops, all 300 views), run on a rented GPU:
2D Gaussian splatting with [gsplat](https://github.com/nerfstudio-project/gsplat) (Apache-2.0, v1.5.3,
our training loop after its reference trainer, masks as transparency) and NeuS-facto in
[hummat/sdfstudio](https://github.com/hummat/sdfstudio) (Apache-2.0, commit `12a3f75`, foreground-mask
loss). Results: see below once measured.

## How established pipelines differ

| | Cameras | Geometry signal | Surface model | Aim |
| --- | --- | --- | --- | --- |
| Meshroom / AliceVision (MPL-2.0) | own structure from motion | patch matching per view (semi-global, needs CUDA) | Delaunay tetrahedra and graph cut | mesh |
| COLMAP, OpenMVS | own | PatchMatch stereo per view | Poisson or graph cut, then mesh refinement | mesh |
| nerfstudio (Apache-2.0) | external | re-rendering loss | none (density or splats) | novel views |
| SDFStudio, NeuS family | external | re-rendering loss | signed distance function | mesh |
| Augenblick (`porto-photogrammetry`, CC BY-NC) | COLMAP or VGGT, then an exact turntable rig | re-rendering loss with surface penalties (SuGaR, 2DGS, PGSR, Gaussian Wrapping) | flattened splats, hull start | mesh |
| Crisp3DS | own turntable solver | patch matching per view | silhouette hull with a smoothed correction | mesh |

What they have and we do not: a surface model that is optimised as a whole (signed distance function,
flattened splats) or a global surface extraction over the depth maps (graph cut). Our voxel fusion
averages per-view depths, so their noise becomes lumps; our best results came exactly when a surface
was imposed (hull, smooth correction field, mesh refinement). The Augenblick notes report the same
disagreement between held-out image quality and geometric accuracy that we measured with Brush.

Licences rule several options out as code: the original Gaussian-splatting code and 2DGS, SuGaR and
PGSR are research-only; DUSt3R, MV-DUSt3R and VGGT weights are non-commercial. gsplat, Brush,
SDFStudio and the original NeuS are permissive. None of this is patent clearance.

## Plan

1. **Photometric refinement of the hull as a pipeline stage** (Rust, WebGPU), with the distinct-peak
   acceptance, behind a setting; gates: no loss on the mustard bottle beyond the fit, the drill's gain
   kept, six single-turn objects unchanged (it starts from their fused mesh there, not from a hull).
2. **Silhouette-only mode** that skips the main matching: the fast path, and the base of 1.
3. **Reference numbers** from the two GPU methods decide whether a signed-distance or splat
   optimiser is worth porting to WebGPU.
4. **Surface extraction** over existing depth (graph cut or similar) as the alternative to voxel
   averaging for single-turn captures.
