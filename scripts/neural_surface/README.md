# Neural surface prototype (MLX)

A research prototype, not part of the pipeline: the reference surface method (NeuS-facto, see
[docs/SURFACE-METHODS.md](../../docs/SURFACE-METHODS.md)) ported to [MLX](https://github.com/ml-explore/mlx) on
Apple GPUs, so that its results can be reproduced here before native versions (WebGPU in the Rust engine, and
MLX through its C API) are written against a fixed recipe. The shipped pipeline must not depend on Python.

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

```
python pack_scene.py INPUTS RGB_DIR MASK_DIR HULL_MESH.stl PACK 2
SNAPSHOT=1000 python neus_facto_mlx.py PACK OUT
```

## MLX pitfall

`mx.value_and_grad(f)` where `f` returns a tuple (a scalar loss and further outputs): gradients of an *outer*
differentiation through the further outputs are zero (MLX 0.31.1 and 0.32.3; the documentation does not say so).
Computing the distance and its gradient that way made the whole surface training wrong: the colour and opacity
terms received no gradient through the distance. Two correct forms: a forward pass plus `mx.grad` (default), or one
`mx.vjp` call (`GRADIENT=vjp`). `mlx_aux_repro2.py` shows the zero gradient, `mlx_vjp_check.py` the one-pass form.

## Results so far (power drill, 300 photos, own cameras and masks)

F1 at 0.5 / 1 / 2 % of the scan's diagonal, each mesh with its own similarity fit to the scan (evaluation only):

| | F1 | |
| --- | --- | --- |
| Reference NeuS-facto, 20 000 steps (Tesla T4, 118 min) | 0.892 / 0.964 / 0.992 | |
| This port, hull start, `SAMPLER=neus`, step 2 000 (Apple M1, about 70 min) | 0.755 / 0.928 / 0.999 | mesh at 256 cubed |
| Same, own `cache` sampler, step 2 000 | 0.224 / 0.510 / 0.794 | sharpness held at 33: rejected |

About 2 s per step on an M1 at 2.1 GiB GPU memory, against 0.35 s per step for the reference on a T4.
