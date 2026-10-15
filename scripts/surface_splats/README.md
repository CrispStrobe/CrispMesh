# 2D Gaussian splatting reference

`splat_reference.py` is a dense PyTorch renderer of 2D Gaussian splats (2DGS) for small scenes, written anew after
[gsplat](https://github.com/nerfstudio-project/gsplat) (Apache-2.0, commit `937e299`: `_torch_impl_2dgs.py`,
`RasterizeToPixels2DGSFwd.cu`, `rendering.rasterization_2dgs`, `utils.depth_to_normal`) and the loss of our trainer
after gsplat's `examples/simple_trainer_2dgs.py`: 0.8 L1 + 0.2 (1 - SSIM) against a background, 0.1 mask L1, 0.05
agreement of the splats' normals with the normals of the median depth, 0.01 depth distortion. Every pixel is
composited over all splats in depth order, restricted to the splats whose 16-pixel tile rectangle covers it, as
gsplat's tiled renderer does; gradients come from autograd.

Its output checks the native WebGPU splats (`crates/dense/src/splats`): forward values, the gradients of the
rendered images and of the projected splats, and the parameters' gradients.

```
python splat_reference.py fixture.npz            # a PyTorch environment
CRISPMESH_GPU_TESTS=1 CRISPMESH_SPLAT_FIXTURE=fixture.npz cargo test --release -p crispmesh-dense --lib splats::
```

Its projection and spherical harmonics equal gsplat's PyTorch implementation to float rounding (checked once by
loading gsplat's `_torch_impl.py` and `_torch_impl_2dgs.py` from a clone).
