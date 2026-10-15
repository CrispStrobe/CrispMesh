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

### Photometric refinement

First a NumPy prototype outside the repository, now a pipeline stage (`photo_refine`, `crisp3ds-dense refine`;
see the crate README). Anchor vertices of the hull mesh search
along their normal for the offset at which the photos agree: a 5 x 5 patch on the mesh's tangent
plane is projected into the most frontal view that sees the vertex and into up to five views 8 to
35 degrees away; the score is the mean normalised cross-correlation with the reference patch.
Visibility comes from the mesh. Accepted offsets are spread over the mesh as a smooth field (weighted
diffusion), the mesh moves inward only, and the search repeats with a smaller range (6, 3, 1.5
voxels). About 100 s for the three rounds on 300 half-size photos in NumPy; 3 s per round natively.

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

### As a pipeline stage

`photo_refine` (0 never, 1 always, 2 when the silhouette comes first: the default) runs after the surface
stage on `mesh/mesh.stl`, keeps the mesh before it as `mesh-unrefined.stl` and writes `refine.json`.
Distinct-peak acceptance (`photo_refine_peak`, 0.15), 120 000 anchors, six views per anchor, three rounds,
inward only when the silhouette comes first. Photos are cropped to the mesh's projection and reduced so
that the object is at most about 1100 pixels across.

| One command, no settings, 300 photos | Before refinement | After | mesh |
| --- | --- | --- | --- |
| Drill | 0.818 / 0.921 / 0.998, mean 0.0033 | 0.857 / 0.977 / 0.999, mean 0.0026 | closed, genus 1 |
| Mustard | 0.766 / 0.918 / 0.997, mean 0.0037 | 0.737 / 0.913 / 0.995, mean 0.0039 | closed, genus 0 |

From the bare silhouette mesh the stage gives 0.867 / 0.962 / 0.998 (drill) and 0.756 / 0.909 / 0.999
(mustard): the fused depth before it is not needed for this result. The three rounds take 9 s on 300 views;
reading the 300 photos again takes 70 to 90 s (they are not yet shared with the earlier stages). Single-turn
captures are untouched by the default (Bunny and Lucy byte-identical). Not run in a browser.

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

Two published surface methods on both captures (crops around the object at half size, 544 x 576 and
432 x 704 pixels, all 300 views, our masks and cameras), run on a rented Tesla T4:

- 2D Gaussian splatting with [gsplat](https://github.com/nerfstudio-project/gsplat) (Apache-2.0, v1.5.3):
  our training loop after its reference trainer, 30 000 steps, masks as transparency, initialised from
  our silhouette mesh, every tenth view held out; mesh from its median depths by plain voxel fusion (512
  cubed) and marching cubes.
- NeuS-facto in [hummat/sdfstudio](https://github.com/hummat/sdfstudio) (Apache-2.0, commit `12a3f75`),
  20 000 steps, foreground-mask loss, pure-PyTorch hash grid (no tiny-cuda-nn); mesh by its marching cubes
  at 1024 cubed.

| | Drill | mean | Mustard | mean | time per object |
| --- | --- | --- | --- | --- | --- |
| Ours, no settings (photos at full size) | 0.857 / 0.977 / 0.999 | 0.0026 | 0.737 / 0.913 / 0.995 | 0.0039 | 10 to 12 min, Apple M1 |
| 2D Gaussian splatting | 0.881 / 0.947 / 0.992 | 0.0026 | 0.791 / 0.899 / 0.969 | 0.0041 | 15 to 21 min training, T4 |
| NeuS-facto | 0.892 / 0.964 / 0.992 | 0.0024 | 0.789 / 0.909 / 0.980 | 0.0036 | 118 min training, T4 |

The scores are close; the meshes are not. Rendered side by side, NeuS-facto is in a different class: one
clean surface with the drill's vents, chuck ridges, trigger and housing seams, and a smooth, correctly
shaped bottle. 2D splatting shows the same relief with a rougher skin. Ours has the right overall shape
and none of that relief. F1 at 0.5 % of the diagonal (about 1.4 mm on the drill) does not measure it.
Two weaknesses of the reference methods show as well: the printed label of the bottle comes out as
shallow relief (lettering embossed into the surface, appearance mistaken for shape), and the underside
no camera saw is rough or open, where our flat base is clean. Held-out views of the splats: 25.7 and
24.3 dB inside the mask.

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

1. **Done: photometric refinement as a pipeline stage** (Rust, CPU), automatic when the silhouette
   comes first. Forced on for single-turn captures it fails the six-object gate (Dragon and Happy Buddha
   lose 0.016 at 0.5 %, Lucy gains 0.012), so it stays off there. Open: sharing the decoded photos with
   the earlier stages, a browser run.
2. **Silhouette-only mode** that skips the main matching: with the refinement after it, the fast path
   to the same result on captures from several heights (about 3 to 4 minutes instead of 10 to 12).
3. **A surface optimised as a whole is the quality target.** NeuS-facto shows what these photos
   contain; nothing in our pipeline reaches that relief. Both kinds (a signed-distance surface optimised by
   re-rendering, and 2D splats) are per-scan optimisers, not models to download; both are wanted, on WebGPU and
   on MLX, natively. First step: the reference model ported to MLX as a prototype
   ([scripts/neural_surface](../scripts/neural_surface/README.md)), started from our silhouette hull with the
   original NeuS sampler limited to a band around the hull, and a smaller network (2 x 64). After the whole
   schedule (20 000 steps, 3.9 h on an M1) the drill scores 0.944 / 0.979 / 0.995 (mean 0.0017) against the
   reference's 0.892 / 0.964 / 0.992 (0.0024), and side by side it shows the same relief. Second step: the
   same model on WebGPU in the engine (`crisp3ds-dense surface`, [crate README](../crates/dense/README.md)),
   with hand-written backward kernels checked against the prototype number for number, 0.25 s per step on the
   M1 (MLX 0.6 s): the whole schedule in 95 minutes, 0.932 / 0.976 / 0.995 (mean 0.0019), the same relief.
   It reads a scene straight from an inputs directory, without Python. Mustard bottle from its inputs: 0.795 /
   0.907 / 0.965 (reference 0.789 / 0.909 / 0.980). 2D splats on WebGPU (`crisp3ds-dense splats`), checked against a
   PyTorch reference of gsplat's 2DGS: drill 0.877 / 0.944 / 0.992, held-out 25.44 dB (gsplat 0.881 / 0.947 / 0.992,
   25.71 dB), about 5 times slower per step than gsplat on a T4. Next: speed, a run setting for both.
4. **Surface extraction** over existing depth (graph cut or similar) as the alternative to voxel
   averaging for single-turn captures.
