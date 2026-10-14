# Neural surface prototype (MLX)

A research prototype, not part of the pipeline: the reference surface method (NeuS-facto, see
[docs/SURFACE-METHODS.md](../../docs/SURFACE-METHODS.md)) ported to [MLX](https://github.com/ml-explore/mlx) on
Apple GPUs, so that its results can be reproduced here before native versions (WebGPU in the Rust engine, and
MLX through its C API) are written against a fixed recipe. The shipped pipeline must not depend on Python.

The native version is `crisp3ds-dense surface` (WebGPU, [crate README](../../crates/dense/README.md)), checked
against this prototype with `export_fixture.py`; on the drill it reaches 0.932 / 0.976 / 0.995 in 95 minutes.

The model follows the configuration of `neus-facto` in
[hummat/sdfstudio](https://github.com/hummat/sdfstudio) (Apache-2.0, commit `12a3f75`), which follows NeuS
(Wang et al. 2021; [code](https://github.com/Totoro97/NeuS), MIT). The code here is written anew from those
descriptions and configurations; the sampler option `neus` follows the hierarchical sampling of the NeuS paper.

## Files

| File | Purpose |
| --- | --- |
| `pack_scene.py` | Packs a recovered scene: cropped colour photos, masks, cameras, bounding sphere, samples of our silhouette mesh. No scan, no supplied poses. |
| `neus_facto_mlx.py` | Training and mesh extraction. Every departure from the reference is a switch; see its header. |
| `check_regression.py` | Fits the geometry network to a known field, with and without gradients of gradients. |
| `diag_samples.py` | Where the samples of a checkpoint fall relative to the network's own surface. |
| `diag_sharpness.py` | Loss of a checkpoint at several fixed sharpness values, geometry held fixed. |
| `profile_nf.py` | Time of the parts of one step. |
| `mlx_aux_repro.py`, `mlx_aux_repro2.py`, `mlx_vjp_check.py` | The MLX gradient pitfall below, in 20 lines each. |
| `export_fixture.py` | Reference values for the native port: the prototype's own model code on fixed rays, with every gradient. |
| `time_variants.sh` | Seconds per step of several switch settings. |

```
python pack_scene.py INPUTS RGB_DIR MASK_DIR HULL_MESH.stl PACK 2
SNAPSHOT=1000 python neus_facto_mlx.py PACK OUT
```

## MLX pitfall

`mx.value_and_grad(f)` where `f` returns a tuple (a scalar loss and further outputs): gradients of an *outer*
differentiation through the further outputs are zero (MLX 0.31.1 and 0.32.3). This is not a faulty operation: MLX
wraps every output after the loss in `stop_gradient` (`value_and_grad` in `mlx/transforms.cpp`) so that it needs a
cotangent for the loss only, and that `stop_gradient` stays in the graph; the documentation does not mention the
consequence for nested differentiation. Computing the distance and its gradient that way made the whole surface
training wrong: the colour and opacity terms received no gradient through the distance. Two correct forms: one
`mx.vjp` call (`GRADIENT=vjp`, the default, one pass) or a forward pass plus `mx.grad` (`GRADIENT=separate`).
`mlx_aux_repro2.py` shows the zero gradient, `mlx_vjp_check.py` the one-pass form.

## Results so far (power drill, 300 photos, own cameras and masks)

F1 at 0.5 / 1 / 2 % of the scan's diagonal, each mesh with its own similarity fit to the scan (evaluation only).
Trials stop at step 2 000 of the 20 000-step schedule; meshes at 256 cubed. Time on an Apple M1, including the
hull fit; one job at a time.

| Setting | Step 2 000 | s/step | Time |
| --- | --- | --- | --- |
| Reference NeuS-facto, 20 000 steps (Tesla T4) | 0.892 / 0.964 / 0.992 (final) | 0.35 | 118 min |
| Reference width (256), two-pass gradient, whole ray, 128 samples | 0.755 / 0.928 / 0.999 | 2.3 | 80 min |
| Width 64, one-pass gradient | 0.815 / 0.966 / 0.997 | 0.85 | 31 min |
| ... and 48 of the 128 samples rendered (learned sharpness) | 0.253 / 0.488 / 0.798 | 0.55 | rejected |
| ... and 48 rendered, drawn at sharpness 512 | 0.655 / 0.892 / 0.979 | | rejected |
| Width 64, one-pass gradient, hull band 0.05, 32 + 4 x 8 samples (default) | 0.848 / 0.976 / 0.998 | 0.60 | 22 min |
| Own `cache` sampler (reference width) | 0.224 / 0.510 / 0.794 | | rejected |

At step 2 000 the meshes have the drill's shape and some relief (vent grid, trigger, seams), clearly less than
the reference's after 20 000 steps. Splitting the batch into parts to bound memory costs no time.

The whole schedule with the default switches (one run, Apple M1, 0.6 s per step; 3 h 55 min wall time including the hull fit, the snapshots
and the 512-cubed mesh, of which about 50 min the machine stalled the process; 1.3 GiB):

| Step | F1 0.5 / 1 / 2 % | Mean distance (fraction of the diagonal) |
| --- | --- | --- |
| Reference NeuS-facto, 20 000 (Tesla T4) | 0.892 / 0.964 / 0.992 | 0.0024 |
| 5 000 | 0.886 / 0.981 / 0.997 | 0.0025 |
| 10 000 | 0.929 / 0.982 / 0.998 | 0.0020 |
| 15 000 | 0.929 / 0.980 / 0.997 | 0.0019 |
| 20 000 | 0.944 / 0.979 / 0.995 | 0.0017 |

Rendered side by side with the reference, the final mesh shows the same relief (trigger, switch bars, seams, vent
grid, battery latch) with similar edge softness; the reference carries more of the printed label as false relief.
Neither shows the chuck's knurling or the lettering that the scan has.
