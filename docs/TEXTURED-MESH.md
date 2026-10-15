# Photo-textured meshes

The native pipeline can export a self-contained GLB with UV coordinates and an
embedded photo atlas. This is an opt-in first implementation; it preserves the
input geometry and does not resolve the outstanding loss of facial relief.

```sh
crispmesh-dense run --photos PHOTOS --calibration LENS.json --output RUN --texture
# Or texture an existing result using its own recovered cameras:
crispmesh-dense texture --inputs RUN/frontend/inputs --mesh RUN/mesh/mesh.stl \
  --output TEXTURE --views 12 --tile-size 1024
```

`mesh.glb`, `atlas.png` and a coverage report are written under `RUN/texture`
(or `TEXTURE`). Studio offers **Export textured GLB (experimental)** for built-in
native/browser engines; completed runs offer GLB download/share. The surface-history
viewer displays untextured geometry; the on-request texture preview below
can switch between photo appearance and grey shading. JSON run options accept
`"texture": true`; the default is false. The browser package builds, and the
pipeline with textures passes the native memory-tree test. A local threaded
Chrome run on M1 (two threads, 60 drill photos, our SAM masks and cached RGB
feature matches, cameras re-solved in the browser) completed all stages including
texture in 95.9 seconds with 1.24 GiB WebAssembly linear-memory high water.
All 60 camera rows contained original RGB sources; twelve were selected for
texturing. This is one execution check, not acceptance of the drill's geometry
or a browser/GPU RSS measurement. Real-device sharing is not yet checked.

New photo-stage scenes include `texture_image` in each camera row: original
capture RGB, undistorted into `texture_images/` with the recovered lens. Texture
export prefers this image; feature gamma/CLAHE stays confined to matching inputs.
Older scenes without the optional field remain readable and use `image`, which
may have been contrast-enhanced. Re-run their photo stage to obtain original
colour. `original_rgb_views` in the texture report counts selected views with
this separate source. This correction does not change cameras, depth matching,
or STL geometry; captured lighting and view seams remain.

The exporter selects up to twelve evenly spaced recovered views. It rejects
back-facing, masked-out and occluded candidates using the reconstructed mesh's
perspective-correct depth buffer at 2048 pixels on the longest side. Triangle
corners and centres are checked. A smoothed normal field makes photo selection
more coherent; it does not move mesh vertices. Each selected photo is cropped
to its own foreground bounds and packed in a padded rectangular atlas tile.
Triangles receive projected UVs, with vertex splits at photo seams. Unobserved
faces point to a neutral-grey tile. Reports include untextured surface area.
`ATTRIBUTION.txt` beside the input cameras, or at a pipeline run root when
writing its `texture` subfolder, is copied and embedded in GLB copyright metadata.
Source licensing must be supplied by the caller; the exporter does not infer it.
No scanner mesh, supplied dataset poses/depth/masks, or generated appearance is
used. No new third-party library or algorithm source was incorporated.

The atlas contains captured colour **and lighting**, not recovered intrinsic
albedo. Discrete view selection still leaves seams, exposure/shadow differences
and projection errors where geometry is wrong. It has no seam levelling,
multiband blending, photometric calibration, efficient chart packing, normal or
roughness estimation. Long triangles can distort projective UV interpolation.
Texture resolution does not establish geometric resolution. Mesh coordinates
and winding are preserved; physical scale and orientation remain those of the
pipeline and are not established by GLB export.

## What KIRI publicly describes

KIRI's [Photo Scan](https://www.kiriengine.app/features/photo-scan) describes
cloud photogrammetry with mesh and texture export. Its
[API](https://docs.kiriengine.app/photo-scan/image-upload/) exposes separate model
quality and texture quality (1K–8K), masking, and texture smoothing, which its
documentation says can trade sharpness for coherent textures. Its separate
[Featureless Object Scan](https://www.kiriengine.app/features/featureless-object-scan)
is described as NeRF-derived neural surface reconstruction producing meshes.
These are vendor descriptions; they do not disclose the exact solver, training
recipe or meshing/texturing implementation. We have not compared identical
inputs in KIRI, and have not uploaded these photos to that service.

KIRI also recommends multiple capture heights and detail photographs. Our
one-ring captures leave unseen surfaces and weak stereo directions; extra
texture pixels cannot fix that. A fair comparison needs the same input photos,
fixed geometric evaluation and matched views of both textured and untextured
meshes. A texture or normal map can convey detail in a rendered view without
that detail being present in the printable mesh.

## Further work

- Texture: correct exposure/seams, improve visibility confidence, select sharper
  photos, and pack connected charts more efficiently. MIT-licensed
  [xatlas](https://github.com/jpcy/xatlas) is one possible UV-library reference,
  not currently a dependency and not a patent clearance.
- Geometry: local matching/camera accuracy on eye and nose regions remains the
  priority; the independent plane experiments have only modest gains. Evaluate
  untextured geometry separately from appearance.
- Robustness: photo-derived masks and valid camera recovery must precede
  texturing. Mustard, light-coloured objects and printed/glossy objects expose
  different front-end failures; do not substitute dataset truth to obtain a
  plausible-looking textured result.

## Optional seam reduction and preview

The standalone exporter accepts `--coherent --color-balance`. Coherent selection
chooses among each triangle's three best independently visible source views,
penalizing source changes across smooth shared edges. It never assigns an
occluded view or fills an unseen region. Colour balancing estimates bounded
display-RGB scalar gains (0.8–1.25) from overlapping visible surface samples; disconnected
view groups keep separate brightness gauges. It preserves hue but does not
remove spatial lighting, estimate albedo, or blend chart boundaries. Both
options are off unless requested.

```sh
crispmesh-dense texture --inputs recovered-inputs --mesh mesh.stl \
  --output texture-review --coherent --color-balance
```

Five local exports (Mario, shoe, rhino, cereal approximation and mustard
approximation) preserved every STL triangle coordinate and winding byte.
Source-image boundaries decreased by 12–17%; rendered improvements were modest,
and visible seams remain. This count is not a perceptual quality score. Cereal
and mustard still have unacceptable geometry; texturing does not fix it.

Studio can preview a completed textured GLB on request. **Photo texture** switches
between photo appearance and grey shading on the same geometry; orientation and
camera stay shared. Closing the preview frees its GPU buffers and atlas. The
preview uses photographed lighting rather than adding a second lighting model.

Using `--views 24` instead of 12 reduces untextured area by only 0.2–1.4
percentage points on these five meshes. Cereal's reviewed views have fewer
grey source boundaries, but shape defects remain. At tile size 1024 the atlas
grows from 4096×4096 to 5120×5120 (56.25% more pixels, about 100 MiB decoded
RGBA before other buffers). Defaults stay at 12 views. All ten GLBs have zero
Khronos validator errors/warnings; the five 24-view atlases have an informational
non-power-of-two image notice.
