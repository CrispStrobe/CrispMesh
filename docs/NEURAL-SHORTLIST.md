# Commercial-friendly neural components: scoped next experiments

Reviewed 2026-09-27. These are source/license findings and proposed experiments,
not trained outputs, App Store clearance, or reconstruction-quality results.
Code, weights, dependencies and input data need separate pinned inventories.

## Priority order

1. **Foreground preparation:** [SAM 2.1 Hiera Tiny](https://huggingface.co/facebook/sam2.1-hiera-tiny).
   Meta explicitly covers code and checkpoints under Apache-2.0 in the
   [official repository](https://github.com/facebookresearch/sam2). Use the
   existing frozen mustard training package and a fixed photo-only box prompt.
   Validate a three-photo smoke test before the 48-view trial. Numerical success
   is not mask acceptance: root reviews label preservation and checkerboard
   leakage. No held-out/reference images are permitted for prompt selection.
2. **Portable splat training:** [Brush](https://github.com/ArthurBrussee/brush),
   reviewed revision `6378a76add3b93501abb55c2dc08d71688537679`, Apache-2.0.
   Its documented COLMAP/Nerfstudio inputs and masks fit the shared camera lane.
   First seal a build/artifact and input preflight; do not start an unbounded
   Rust build under the Mac disk reserve. Compare against
   [Metal msplat](MSPLAT-IOS-AUDIT.md) later using identical permitted inputs.
3. **Camera/geometry initialization:** specifically
   [facebook/map-anything-apache](https://huggingface.co/facebook/map-anything-apache),
   not a similarly named default checkpoint. Inventory the exact model revision,
   weight hash/size and code license before downloading. Large weights belong
   under `/Volumes/backups/ai`, on VPS storage, or in an approved remote GPU
   environment. Compare inferred
   cameras and supported geometry against the classical lane; predictions are
   neither measured depth nor a complete accepted mesh.

The iPhone 13 mini remains an RGB capture client. Neither rear LiDAR nor an
on-phone neural model is required. Learned foreground masks, camera initialization
and appearance training are independent experiments, not one untested pipeline
whose combined changes could conceal the source of improvement or regression.

## License and naming traps

- [rayanht/msplat](https://github.com/rayanht/msplat) is an Apache-2.0 Metal
  trainer. [pointrix-project/msplat](https://github.com/pointrix-project/msplat/blob/71732d50ce286062b411086b48de5fe59a1497d2/LICENCE)
  is a different CUDA rasterizer with noncommercial/research restrictions.
- [MVSplat](https://github.com/donydchen/mvsplat/blob/main/LICENSE) has MIT root
  code, but its [required rasterizer](https://github.com/dcharatan/diff-gaussian-rasterization-modified/blob/main/LICENSE.md)
  restricts commercial use; separate checkpoint terms were not verified. Do not
  ship the default stack under a blanket MIT assertion.
- [Nerfstudio/Splatfacto](https://docs.nerf.studio/nerfology/methods/splat.html)
  with [gsplat](https://github.com/nerfstudio-project/gsplat) is a useful
  Apache-licensed CUDA comparison path. Gaussian export is not a mesh. The
  separate rendered-depth/TSDF route requires its own geometry validation.
- [Depth Anything V2 Small](https://github.com/DepthAnything/Depth-Anything-V2#license)
  is an Apache-licensed relative-depth candidate, including an
  [Apple Core ML conversion](https://huggingface.co/apple/coreml-depth-anything-v2-small).
  Larger checkpoint licenses differ. MoGe-2 small is MIT-tagged on its author
  model page, but sparse weight-license documentation needs clarification before
  redistribution. Neither replaces foreground segmentation or multi-view checks.
- [Apple SHARP model terms](https://github.com/apple/ml-sharp/blob/main/LICENSE_MODEL)
  exclude commercial product development. Original VGGT-1B weights are
  noncommercial; its separate commercial checkpoint has additional conditions,
  not the simple Apache grant of the MapAnything checkpoint above.

The supplied [RichardForests collection](https://huggingface.co/collections/RichardForests/3d-4d-gaussian-splatting)
and [jnfang collection](https://huggingface.co/collections/jnfang/3dgsplats)
are paper-discovery lists, not collectively licensed model bundles. Follow each
paper to its actual code, rasterizer and checkpoint terms before adoption.

## Acceptance and resources

Sol agents own scoped implementation and synthetic regression tests; root reviews
source, live preflight and artifacts. Preserve originals and failed candidates.
Keep the 10 GiB Mac reserve; use `/mnt/storage` for large assets and
`/mnt/volume1` for fast scratch only after checking available space. Do not use
`/tmp`. A staging manifest is not a completed download or inference result.
Any Kaggle submission separately requires the local usage guide and quota checks.
On the Mac, large reconstruction artifacts use `/Volumes/backups/code/crispmesh-data`
and weights use `/Volumes/backups/ai`; see [storage policy](STORAGE.md).
