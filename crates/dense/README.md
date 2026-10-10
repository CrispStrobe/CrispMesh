# crisp3ds-dense: the native dense pipeline

The reconstruction from turntable photos to a checked STL in Rust, without
Python: GPU stages as WebGPU compute shaders through `wgpu` (Metal, Vulkan,
DirectX 12, and WebGPU in browsers), the rest as plain Rust. It runs as one
command (`run`), as a library call, through a C interface and, for the dense
stages, in a browser; every stage is also a subcommand. With the default
providers (threshold masks, our own turntable solver) the whole way from
photos to the STL starts no other program.

This crate began as a port of `scripts/turntable_mesh` and is now the primary
implementation. New behaviour is developed here and judged by the scanner
evaluator on the four test objects. The Python package remains as the
reference for what was ported, and is not extended.

| Part | Where it runs |
| --- | --- |
| Photos to scene (`photos`): capture order, threshold masks, hole cleanup, contrast images, lens handling, audit and gates, undistortion, scene | this crate |
| Camera providers `turntable` (our own solver, the default), `markers` (printed mat), `import` | this crate |
| Mask providers `threshold` (the default), `import` | this crate |
| Camera provider `colmap` | external program: COLMAP (BSD-3-Clause), started as a child process |
| Camera provider `alicevision` | external programs: AliceVision (MPL-2.0), started as child processes |
| Mask provider `external-sam` | external program: SAM 2.1 in Python with PyTorch (Apache-2.0, BSD-3-Clause) |
| Mask provider `sam` (feature `sam-onnx`, off by default) | this crate (`src/photos/sam/`); the network runs in ONNX Runtime (MIT), loaded as a shared library at run time through the `ort` crate; the model (SAM 2.1 Hiera-tiny, Apache-2.0) from `--sam-model DIR` or fetched once from Hugging Face |
| Inputs from a scene (`inputs`), dense stereo (`stereo`, GPU), surface (`mesh`), photo check (`check`) | this crate |
| Run driver, events, cancellation, settings schema, start points and provider description | this crate |
| In a browser (`web/`) | this crate as WebAssembly: from photos with the providers that start no program (threshold or imported masks; turntable, markers or imported cameras), or from an inputs folder |
| Scanner evaluator (`scan_evaluate.py`) | Python, a development tool; never an input of any stage |

Where the stages sit among the providers and which platforms run what is
described in [`docs/ARCHITECTURE.md`](../../docs/ARCHITECTURE.md).

## Rules

- **Native first.** New behaviour is written here. It gets a setting when it
  changes results, the setting is listed in `config::NATIVE_ONLY`
  (`support_from_sparse` so far), and it is kept only when the scanner scores
  (`scan_evaluate.py`) on the four objects say so; the numbers are in this file.
- **What was ported stays comparable.** A ported stage reads and writes the
  files of its Python counterpart (the inputs directory with `cameras.json`,
  masks and `sparse_points.npy`; `depths.npz`, `volume.npz`, `mesh/mesh.stl`,
  `result.json`, `events.jsonl`, `config.json`), so a run may mix Python and
  native stages; that is how each stage was checked, and the measured parity is
  listed per stage below.
- **Same settings.** `DenseConfig` has the settings of `dense_config.py` with
  the same defaults; a test fails when they drift
  (`tests/fixtures/dense-config-defaults.json`). Native-only settings are the
  stated exception.
- **Same events.** `docs/ENGINE-CONTRACT.md` is binding.
- **No reference geometry** is ever read by a stage.

## Layout

| Path | Content |
| --- | --- |
| `src/config.rs` | Settings mirror |
| `src/events.rs` | Event log writer |
| `src/npz.rs`, `src/stl.rs` | Array and mesh file formats |
| `src/mesh/` | Surface extraction (port of `tsdf_hull_mesh.py`) |
| `src/gpu/`, `src/shaders/` | `wgpu` device handling and WGSL kernels |
| `src/storage.rs` | Files on disk or in an in-memory tree (`mem:` paths; everything in a browser) |
| `web/` | Browser package: `wasm-bindgen` bindings, JavaScript wrapper, worker, test page and its headless test |
| `src/run.rs`, `src/control.rs` | The run driver (port of `dense_pipeline.py`): library API and `run` command; cancel flag, deadlines, stage logs |
| `src/capi.rs`, `include/crisp3ds_dense.h` | C interface of the run driver (feature `capi`) |
| `src/scene.rs` | Inputs directory from an AliceVision scene (port of `dense_all_views_inputs.py`) |
| `src/check.rs`, `src/render.rs` | Photo check and its sheets (ports of `mesh_photo_check.py`, `stl_compare_render.py`) |
| `src/inputs.rs` | Inputs directory as `Stereo.__init__` prepares it |
| `src/hull.rs`, `src/repair.rs`, `src/fusion.rs`, `src/stereo/` | Ports of `multiscale_stereo.py` |
| `src/photos/` | From photos to the scene: mask and camera providers, gates, undistortion (port of `photos_to_inputs.py`) |
| `tools/colmap_pycolmap.py` | Stand-in for the `colmap` executable through the PyCOLMAP wheel (development aid) |
| `src/main.rs` | `crisp3ds-dense <stage>` command line |

## Build and test

```sh
cd crates/dense
cargo test
cargo run --release -- defaults
```

Formatting follows `rustfmt.toml` (`cargo fmt --check`), lints are
`cargo clippy --all-targets -- -D warnings`; `.github/workflows/dense-native.yml`
runs both and the tests on Linux, macOS and Windows. Needs Rust 1.88 or newer.

## A whole run: `crisp3ds-dense run`, `crisp3ds_dense::run`, C interface

The driver (it began as a port of `scripts/turntable_mesh/dense_pipeline.py`)
for three starting points, in one process:

```sh
crisp3ds-dense run --output RUN --inputs DIR                                   # an inputs directory
crisp3ds-dense run --output RUN --scene final.sfm --prepared DIR --raw-masks DIR
crisp3ds-dense run --output RUN --photos DIR --calibration lens.json           # plain photos
    [--masks PROVIDER] [--cameras PROVIDER] [options of `crisp3ds-dense photos`]
    [--config FILE] [--set key=value ...] [--threads N] [--stereo-timeout S]
    [--minimum-free-gib G] [--reuse-depths depths.npz] [--no-live-previews]
    [--preview-step N] [--skip-check] [--no-preview] [--keep-volume]
crisp3ds-dense run --describe [tool locations]      # start points, providers, their options and availability, as JSON
crisp3ds-dense settings --schema                    # every setting with group, kind, default and meaning, as JSON
```

The photos stage reads and writes through `crate::storage`, so the run
directory, the photos and the lens file may all be in the in-memory tree
(`mem:` paths, natively too; every path in a browser). There, and in a
browser, a run whose mask or camera provider starts an external program is
refused before anything runs (the provider table says which do). The Bunny's
photos to STL with the default providers entirely in `mem:` gave the same
mesh and cameras, byte for byte, as the same run on disk (1 046 448
triangles, 484 files in both). A host that hands photos over lazily
(`storage::set_memory_source`) also registers `storage::set_memory_lister`,
so that the photo folder can be listed.

From photos the photos stage runs first (its `masks` and `cameras` stages, in
`RUN/photos/`) and its scene is the inputs directory; there is no `inputs`
stage in such a run, because the `cameras` stage writes the scene. A gate that
rejects the cameras ends the run as failed. `--reference` (scoring against a
scan) is refused with a pointer to the Python tool. `--device`, `--python`,
`--torch-python` and `--native` are accepted and ignored, so a command line
written for the Python driver works.

The Bunny from its 73 photos in one command (`--masks threshold --cameras
alicevision`, AliceVision 3.4 local build, 4 threads, other jobs on the
machine): complete, 73 of 73 photos registered, 608 s (photos stage 555, stereo
45, surface 5.4, check 2.4), scanner F1 `above_margin` 0.959 / 0.993 / 1.000
and `all` 0.890 / 0.926 / 0.947 (before `support_from_sparse`, see "Masks
without a network").

**For a host application.** `crisp3ds_dense::run::describe()` (and
`describe_with(photo_options)`, which takes the tool locations as words of the
photos command line) returns what `run --describe` prints: per start point its
fields, the `stages` a run goes through, and for the photos start the provider
choices of each module. Every provider carries `available`, `reason` and
`version` (COLMAP is asked with `colmap help`; AliceVision has no version
option, so the version is read from the names of its libraries; SAM is checked
for its interpreter, source, checkpoint and script without starting Python;
providers in this crate report the crate's version) and its options as
`settings` rows (`name`, `flag`, `kind` among `switch`, `integer`, `number`,
`text`, `choice`, `provider`, `path`, `directory`, `executable`, `path_list`;
`default`, `choices`, `variable`, `repeated`, `meaning`). Options shared by a
module are in the module's `settings`; tools, machine, deadlines and gates are
in the start point's `option_groups`. The rows come from the table the
command-line parser itself reads (`src/photos/option_table.rs`;
`crisp3ds-dense photos --list-options` prints all of it), so a form built from
them cannot drift from the parser. `crisp3ds_dense::photos::availability::check`
is the same check as a function. `config::settings_schema()` is what
`settings --schema` prints.

The run directory is the one `docs/ENGINE-CONTRACT.md` describes: `config.json`,
`events.jsonl` (`run_started`, stage events, artifacts, `run_finished`; once
`run_started` is written `run_finished` always follows, also on failure, with an
`error` event before it), `pipeline.json` with the Python driver's keys,
`input-sheet.png`, `stereo/`, `mesh/`, `check/`, one `<stage>.log` per stage.
Stages run on the calling thread; preview volumes are meshed on a worker thread
while matching continues, and their `preview_mesh` events arrive when they are
done. Creating a file named `cancel` in the run directory (or setting the
cancel flag of the library call) stops the run: the stereo stage at the next
view of mask repair or matching, surface extraction before each pass over the
grid, the check before each view, and a preview mesh in progress likewise, so
the call returns without waiting for it. Measured on the Bunny, cancelling at
seven moments between the first preview volume and the last: 0.2 to 1.1 s until
the process had ended. The run then ends with status `cancelled`. `stereo/volume.npz` is deleted at the end unless
`--keep-volume`. `scripts/turntable_mesh/export_replay.py` accepts the result.

Against the Python driver (all stages in Python, Torch on MPS) on the same
machine: on the synthetic sphere and on the Bunny started from its scene, the
event logs have the same stage sequence, the same artifact kinds and counts per
stage (Bunny: 4 depth sheets, 4 preview volumes, 4 preview meshes, 3 reports,
one of each other kind) and the same metric names, and `pipeline.json` has the
same keys. Synthetic: 33 792 triangles from both, 21.0 s / 1.3 s. Bunny:
1 046 354 / 1 047 200 triangles, both closed, silhouette IoU median 0.9667 /
0.9667, scanner F1 `all` 0.905 / 0.941 / 0.955 against 0.905 / 0.941 / 0.956 and
`above_margin` 0.964 / 0.995 / 1.000 against 0.963 / 0.995 / 1.000 (largest
difference 0.0011), 1006 s / 193 s with both measured while other jobs loaded
the machine; a native run from the inputs directory alone took 95 s (stereo 78,
surface 12, check 5) under similar load.

As a library:

```rust
use std::sync::{atomic::AtomicBool, Arc};
use crisp3ds_dense::run::{run, RunOptions};

let options = RunOptions { output: "runs/bunny".into(), inputs: Some("bunny/inputs".into()), ..Default::default() };
let cancel = Arc::new(AtomicBool::new(false));           // set from any thread to stop the run
let observer = Arc::new(|event: &serde_json::Value| {     // every event, on the thread that emits it
    println!("{}", event["type"]);
});
let report = run(&options, Some(observer), Some(cancel))?;   // blocks; call it on a worker thread
```

`RunOptions` deserialises from JSON with the same field names (unknown fields
are an error); `settings` takes values by name as a settings form produces them.
For a run from photos set `photos` and put the photos stage's options into
`photo_options`, one word per element (`["--calibration", "lens.json",
"--masks", "threshold"]`).
`run_async` is the same as a future, for hosts that must not block.
`run` returns the content of `pipeline.json`, or the error that stopped the run
(`crisp3ds_dense::control::Stopped` when cancelled or past a deadline). The
observer is called from the run's thread and from the preview thread, so it must
be `Send + Sync` and should return quickly; a Tauri command would forward each
event to the window from there. One run uses one GPU device; start runs one at a
time.

From C, Swift, Kotlin or C++ (`include/crisp3ds_dense.h`):

```sh
cargo rustc --release --lib --features capi --crate-type cdylib      # or staticlib
```

```c
char *error = NULL;
Crisp3dsRun *run = crisp3ds_run_start("{\"output\": \"runs/bunny\", \"inputs\": \"bunny/inputs\"}", &error);
for (;;) {
    char *state = crisp3ds_run_poll(run);    /* {"finished", "status", "events": [...], "report", "error"} */
    /* ... show the new events; stop when "finished" is true ... */
    crisp3ds_string_free(state);
}
crisp3ds_run_cancel(run);                    /* optional, from any thread */
crisp3ds_run_free(run);                      /* cancels if still running, waits, releases */
```

The run executes on a thread the library starts; `poll` never blocks. This is
the only module with `unsafe` code beyond byte casts (it dereferences the
caller's pointers).

## From photos to the scene: `crisp3ds-dense photos`

Native form of `python -m scripts.turntable_mesh.photos_to_inputs`, built
around providers (`docs/ARCHITECTURE.md`, `docs/PHOTOS-TO-INPUTS.md`): one for
the masks, one for the cameras; staging, hole cleanup, contrast images, gates,
undistortion, the scene, sheets and events are done in this crate for all of
them. No Python is needed with `--masks threshold` or `import:DIR` and
`--cameras alicevision` (install prefix), `colmap` (executable) or
`import:PATH`.

```sh
crisp3ds-dense photos --photos DIR --calibration lens.json --output DIR [--events events.jsonl] \
    --masks threshold|import:DIR|external-sam --cameras alicevision|colmap|import:PATH [options]
crisp3ds-dense photos --list-providers
```

`--list-providers` prints each provider's platforms (desktop, mobile, wasm),
the external programs it starts and its license as JSON. Same gates, exit
codes (0 complete, 2 cameras rejected, 1 a step failed), events (`masks`,
`cameras`; `mask_sheet`, `sparse_overlay`, `report`) and layout as the
reference, except that the scene's undistorted photos are in `inputs/images/`
(there is no `native-prepared/`) and that `prepareDenseScene` is not called.

| Step | Where (`src/photos/`) | Reference |
| --- | --- | --- |
| Capture order, `capture_NNNN.png` copies, coarse dark-object masks | `staging.rs`, `coarse.rs` | `step_coarse`, `coarse_mask`, `otsu_threshold`; components labelled like `scipy.ndimage.label` |
| Mask providers `threshold`, `import`, `external-sam` | `masks.rs` | `segment.py` is started as a child process for `external-sam` |
| Dark-hole cleanup, published masks, contact sheet | `cleanup.rs`, `sheets.rs` | `silhouette_cleanup.py`, `step_publish_masks` |
| Contrast images | `contrast.rs` | `contrast_image`: OpenCV's 8-bit RGB/Lab conversions and CLAHE, reimplemented |
| Lens file, scaling, locked AliceVision intrinsic | `calibration.rs` | `load_calibration`, `scale_calibration`, `calibrated_scene` |
| Camera providers `alicevision`, `colmap`, `import` | `cameras.rs`, `process.rs` | `build_commands`, `bounded`; COLMAP is new |
| Neutral solution; readers for `.sfm` and COLMAP models | `solution.rs` | |
| Audit, ring statistics, gates | `audit.rs`, `ring.rs` | `alicevision_cameras.py`, `ring_statistics`, `decide_gates` |
| Undistortion and scene | `scene_writer.rs` (map and mask remap from `src/scene.rs`) | `prepareDenseScene` and `dense_all_views_inputs.py` |
| Sparse overlay, stages, `frontend.json` | `sheets.rs`, `run.rs` | `step_overlay`, `run` |

Parity with the reference, measured on the 3DLF photos (73 per object,
1749 x 1155; OpenCV 4.10, SciPy, Pillow on the reference side):

- **Masks.** Coarse masks plus cleanup: identical to the Python functions in
  all 73 masks of the Bunny and of the Armadillo (0 of 147 M pixels).
  `masks-report.json` is identical on the Dragon and Lucy.
- **Contrast images.** Identical to `contrast_image` (gamma 0.5, CLAHE 2.0 on
  an 8 x 8 grid) in all 292 photos of the four objects: 0 of 442 M channel
  values per object. The two colour conversions are identical to
  `cv2.cvtColor` for all 2^24 input triples in both directions
  (`tests/fixtures/dense-native/make_photos_fixtures.py --exhaustive`). They are
  OpenCV's integer algorithms (`RGB2Lab_b`, `Lab2RGBinteger`) with tables
  computed here; one entry of the cube-root table (324) lands on a rounding tie
  in single precision that OpenCV resolves the other way and is set explicitly.
- **Audit and gates.** `camera-audit.json`, `ring-sanity.json` and `gates.json`
  of the Dragon and Lucy scenes agree with the files the Python stage wrote to
  2.6e-13 relative in every number; the sparse-overlay fractions are equal.
- **Scene.** Importing the existing `final.sfm` of the four objects reproduces
  the existing inputs exactly: cameras (`k`, rotation, translation), sparse
  points and all remapped masks.
- **Undistorted photos** against AliceVision's `prepareDenseScene` (13 photos
  per object): mean absolute difference 0.007 to 0.010 grey levels, 99 % of the
  values identical, at most 0.06 % differ by more than one level, largest
  difference 13 to 17 at dark-to-light edges. Both are bilinear; AliceVision
  converts to linear light first, interpolates in floating point and converts
  back, this crate interpolates the 8-bit values. Bunny with the same cameras
  and masks, native dense stages: F1 `all` 0.9049 / 0.9404 / 0.9554 against
  0.9049 / 0.9412 / 0.9560 with AliceVision's images, `above_margin` 0.9641 /
  0.9948 / 0.9998 against 0.9629 / 0.9946 / 0.9998 (largest difference 0.0012).
- **Sheets** are pictures for people: tiles are reduced with the crate's
  bicubic resampler (Pillow Lanczos, OpenCV bilinear in the reference) and
  labelled with the built-in font, which has no `_` or `.`.

Time for everything that is not a provider, Bunny, Python / native: 70 s / 7 s
(coarse masks 7.1 / 0.7, cleanup 14.2 / 1.4, masks and sheet 4.6 / 1.0,
contrast images 12.6 / 1.7, audit 2.0 / 0.02, undistortion and scene 27.3
(`prepareDenseScene`, check, inputs) / 1.3, overlay 2.0 / 0.4; 4 threads).

### Masks without a network

Scanner F1 (`scan_evaluate.py`, evaluation only) of the native dense stages on
three mask sets with the same cameras: SAM 2.1 masks (the existing ones), the
`threshold` provider at the fixed level 70 and at Otsu's level per photo
(104 to 125 on these photos). `all` / `above_margin` at 0.5 % of the diagonal;
in brackets at 1 % and 2 %.

| | SAM 2.1 | threshold 70 | threshold Otsu |
| --- | --- | --- | --- |
| Bunny `all` | 0.905 (0.941, 0.956) | 0.892 (0.934, 0.952) | 0.891 (0.925, 0.947) |
| Bunny `above_margin` | 0.963 (0.995, 1.000) | 0.957 (0.995, 1.000) | 0.962 (0.994, 1.000) |
| Armadillo `all` | 0.923 (0.971, 0.987) | 0.901 (0.962, 0.984) | 0.907 (0.957, 0.972) |
| Armadillo `above_margin` | 0.952 (0.996, 1.000) | 0.930 (0.989, 0.999) | 0.950 (0.997, 1.000) |
| Dragon `all` | 0.785 (0.928, 0.981) | 0.786 (0.914, 0.972) | 0.766 (0.895, 0.956) |
| Dragon `above_margin` | 0.834 (0.963, 0.994) | 0.851 (0.969, 0.995) | 0.848 (0.967, 0.994) |
| Lucy `all` | 0.790 (0.904, 0.938) | 0.807 (0.923, 0.956) | 0.818 (0.936, 0.962) |
| Lucy `above_margin` | 0.826 (0.945, 0.970) | 0.845 (0.967, 0.991) | 0.862 (0.985, 1.000) |
| Silhouette IoU against own masks (median), B / A / D / L | 0.967 / 0.935 / 0.949 / 0.881 | 0.951 / 0.935 / 0.947 / 0.909 | 0.967 / 0.942 / 0.951 / 0.928 |
| Genus, B / A / D / L | 11 / 2 / 22 / 14 | 4 / 19 / 12 / 3 | 16 / 4 / 24 / 3 |

Reading, with the preview sheets: at level 70 lit upward faces are brighter
than the threshold and are cut away by the silhouette hull (the top of the
Armadillo's shell is sliced off; genus 19). Otsu's level includes them and
brings the Bunny and the Armadillo to SAM's score above the support (within
0.002), and the Dragon and Lucy above it (SAM drops horns, hands and wing
tips in runs of photos; the threshold keeps them). The price is at the base:
the contact shadow is dark, belongs to the largest dark region and becomes a
ragged skirt around the feet, which costs 0.014 to 0.019 in `all` on three
objects. Thin parts are no worse than with SAM on any sheet.

**The base with threshold masks: `support_from_sparse`.** What the shadow costs
was traced on the Bunny: it is not a thin skirt but the level of the flat base.
The masks reach 14 to 17 pixels below the object, the hull follows, dense depth
continues the wall of the object down through the featureless shadow, and the
lowest measured surface, which is where the base is cut, lay 8 voxels (1.3 % of
the diagonal) below the one from SAM masks. The sparse points do not go there:
they are features matched across photos, and a soft shadow has none. The fusion
stage therefore no longer places the support below the lowest sparse points
(all but five, or one in a thousand, which may be wrong matches); a support
above that level is left alone. This setting exists only here (default on; the
Python reference does not have it). Scanner F1 `all` at 0.5 % / 1 % / 2 %,
same cameras, before and after:

| | SAM masks (unchanged by the setting) | threshold, before | threshold, with the setting | support raised by |
| --- | --- | --- | --- | --- |
| Bunny | 0.905 / 0.941 / 0.956 | 0.892 / 0.927 / 0.947 | 0.902 / 0.938 / 0.954 | 6.8 voxels |
| Armadillo | 0.923 / 0.971 / 0.987 | 0.907 / 0.956 / 0.971 | 0.907 / 0.956 / 0.971 | 0 |
| Dragon | 0.785 / 0.928 / 0.981 | 0.765 / 0.895 / 0.957 | 0.780 / 0.915 / 0.974 | 3.8 voxels |
| Lucy | 0.790 / 0.904 / 0.938 | 0.819 / 0.937 / 0.962 | 0.819 / 0.937 / 0.962 | 0 |

With SAM masks nothing changes on any object (the support is never below the
sparse points; all eight numbers per object are identical). `above_margin`
with threshold masks moves by at most 0.0022 (Dragon, 0.8468 to 0.8446 at
0.5 %). The threshold masks are now within 0.0034 of SAM on the Bunny and
above it on Lucy; they remain 0.005 / 0.013 / 0.007 below on the Dragon and
0.015 on the Armadillo. What is left there is not the base level: on the
Armadillo the support is the same for both mask sets (three small contact
points, which the sparse points reach), and the loss is hull that the shadow
keeps beside and under the raised soles and between the Dragon's claws, joined
to the feet, so it is neither thin nor separate. Removing thin unmeasured hull
lying on the support was tried (columns on the support plane, runs no taller
than 4 to 16 voxels): it changed no score by more than 0.001 on the Bunny or
the Armadillo and is not in the crate.

**Contact shadow taken out of the masks (`--threshold-shadow`).** With the
turntable cameras the remaining defect was flat flakes on the support around
and between the feet: contact shadow in the masks. The shadow is darker than
the level but lighter than the object's material, and it lies on the
turntable, so in an upright photo there is turntable below it, not object.
The threshold provider now keeps a mask pixel only when object core (darker
than `--threshold-shadow` of the way from the mask's median grey to the level,
0.25 by default) lies at or below it in its column, then keeps the largest
part. Against SAM masks on every sixth photo the extra area falls from 3.8 to
5.7 % of the object to 2.3 to 3.6 %, and the missing area rises from 0.04 to
1.3 % to 0.2 to 1.5 %. One command from the photos, turntable cameras, scanner
F1 at 0.5 % / 1 % / 2 %:

| | threshold, before | threshold, shadow taken out | SAM masks | after minus SAM, `all` |
| --- | --- | --- | --- | --- |
| Bunny `all` | 0.905 / 0.938 / 0.953 | 0.904 / 0.939 / 0.956 | 0.908 / 0.942 / 0.956 | -0.0049 / -0.0031 / -0.0003 |
| Bunny `above_margin` | 0.967 / 0.997 / 1.000 | 0.964 / 0.995 / 1.000 | 0.966 / 0.996 / 1.000 | |
| Armadillo `all` | 0.913 / 0.960 / 0.975 | 0.924 / 0.972 / 0.987 | 0.926 / 0.971 / 0.987 | -0.0017 / +0.0004 / 0.0000 |
| Armadillo `above_margin` | 0.950 / 0.996 / 1.000 | 0.952 / 0.997 / 1.000 | 0.953 / 0.996 / 1.000 | |
| Dragon `all` | 0.784 / 0.913 / 0.973 | 0.802 / 0.933 / 0.982 | 0.794 / 0.930 / 0.982 | +0.0076 / +0.0026 / -0.0006 |
| Dragon `above_margin` | 0.849 / 0.964 / 0.994 | 0.847 / 0.965 / 0.995 | 0.838 / 0.961 / 0.994 | |
| Lucy `all` | 0.819 / 0.937 / 0.963 | 0.822 / 0.940 / 0.965 | 0.824 / 0.943 / 0.968 | -0.0020 / -0.0023 / -0.0022 |
| Lucy `above_margin` | 0.862 / 0.985 / 0.999 | 0.861 / 0.985 / 0.999 | 0.860 / 0.983 / 1.000 | |

`above_margin` drops by at most 0.0029 (Bunny at 0.5 %, 0.9665 to 0.9636). The
flakes are gone on the preview sheets of the Armadillo and the Dragon, and the
feet, claws and Lucy's base are intact. Because of this `threshold` is the
default mask provider: the default command needs no external program. One run
per variant and object; the Bunny's whole-surface margin at 0.5 % (0.0049) is
the closest to the 0.005 limit.
A core share of 0.35 was tried as well: closer on the Bunny (-0.0026 at
0.5 %), worse on Lucy (-0.0040 / -0.0050 / -0.0029), so 0.25 stays.

With the turntable solver's sparse points (Bunny 6 733, Armadillo 7 853,
Dragon 5 062, Lucy 2 663, counted in the runs with threshold masks)
`support_from_sparse` engages with threshold masks on the Bunny (support
raised by 3.2 voxels), the Dragon (3.2) and the Armadillo (0.6), not on Lucy;
with SAM masks only on the Dragon (0.8).

### SAM 2.1 in this process: `--masks sam`

`src/photos/sam/` ports what `segment.py` does around the network (prompts
from the coarse masks, choice among the network's masks, the region holding
the primary point) and runs SAM 2.1 Hiera-tiny as two ONNX graphs through
ONNX Runtime. It is behind the cargo feature `sam-onnx` (off by default, so
normal builds and CI download and link nothing):

```sh
cargo build --release --features sam-onnx
crisp3ds-dense run --photos data/bunny/rgb --calibration scripts/turntable_mesh/calibrations/3dlf-pro.json \
    --masks sam --sam-runtime /path/to/libonnxruntime.dylib --output runs/bunny-sam
```

The runtime is ONNX Runtime's shared library (`--sam-runtime` or
`ORT_DYLIB_PATH`; the pip wheel's `onnxruntime/capi/libonnxruntime.*.dylib`
works). The model is `--sam-model DIR` (`model.json`, `encoder.onnx`,
`decoder.onnx`, written by `tools/sam2_export_onnx.py`), else it is fetched
once from `huggingface.co/cstr/sam2.1-hiera-tiny-ONNX` into the cache
(`CRISP3DS_CACHE_DIR`, else the platform's) and checked against SHA-256 pinned
in `src/photos/sam/fetch.rs`, with progress in the event log (step
`sam-download`). `--sam-candidates single` (default) takes SAM's own single
mask, `several` the best-scored of its three proposals that agrees with every
point, as `external-sam` does. The dark-hole cleanup budget is 0.05 for the
SAM providers unless given.

Exported graphs against PyTorch on the CPU (8 Bunny photos at 1749 x 1155):
image embedding within 1.5e-5, mask logits within 7.2e-5, the same mask chosen
on all 8 (IoU 1.0). The provider against `segment.py` on the CPU, 73 Bunny
photos: 69 masks identical, lowest IoU 0.999996. Scanner F1 at 0.5 %, whole
surface / above the support, turntable cameras, native dense stages:

| | threshold | SAM, MPS before the fix | `sam` |
| --- | --- | --- | --- |
| Bunny | 0.905 / 0.967 | 0.908 / 0.966 | 0.912 / 0.966 |
| Armadillo | 0.913 / 0.950 | 0.926 / 0.953 | 0.921 / 0.947 |
| Dragon | 0.784 / 0.849 | 0.791 / 0.837 | 0.804 / 0.850 |
| Lucy | 0.819 / 0.862 | 0.824 / 0.860 | 0.809 / 0.848 |

On these dark objects SAM does not beat the threshold: it is an option for
other captures, and `threshold` stays the default.

**Second backend: ggml (feature `sam-ggml`).** The same provider runs SAM 2.1
in CrispEmbed's ggml engine (`src/sam2.cpp` there), opened at run time from
`libcrispembed-sam2` (CrispEmbed's CMake option `CRISPEMBED_SAM2_SHARED`; with
`-DBUILD_SHARED_LIBS=OFF -DGGML_METAL=ON` it is one 2.6 MB library that needs
only system frameworks). The model is a GGUF file given to `--sam-model`
(`cstr/sam2.1-hiera-tiny-GGUF`, F16 63 MB); `--sam-runtime` or
`CRISPEMBED_SAM2_LIB` names the library, `--sam-accelerator gpu` uses the GPU
backend it was built with (Metal on Apple machines), `cpu` the CPU:

```sh
cargo build --release --features sam-ggml
crisp3ds-dense run --photos data/bunny/rgb --calibration scripts/turntable_mesh/calibrations/3dlf-pro.json \
    --masks sam --sam-model sam2.1-hiera-tiny-f16.gguf --sam-runtime libcrispembed-sam2.dylib \
    --sam-accelerator gpu --output runs/bunny-sam
```

Masks of the 73 Bunny photos against the reference script on the CPU (both with
`--sam-candidates several`), and time per photo on an Apple M1 shared with other
jobs (load average 50 to 175 during these runs, so the times are upper bounds):

| Backend | Masks: median / lowest IoU, identical | Time per photo (encoder) | Peak memory |
| --- | --- | --- | --- |
| ONNX Runtime 1.30, CPU, F32 | 1.0 / 0.999996, 69 of 73 | 2.4 s (2.3 s) | 2.4 GB |
| ggml, CPU, F32 | 1.0 / 0.999996, 69 of 73 | 3.3 s (3.2 s) | 0.72 GB |
| ggml, Metal, F16 | 0.99996 / 0.99912, none | 1.5 s (1.3 s; 1.1 s on a quieter machine) | 0.38 GB |

CrispEmbed's own check (`test-sam2-diff`, every encoder stage against PyTorch on
the CPU): F32 and F16 cosine at least 0.999999 on the CPU and 0.999994 on Metal;
Q8_0 drifts (worst mask IoU 0.9895) and Q4_K is unusable, so neither is offered.

**Flat base only on a support (`support_evidence`), unmeasured surface inside
the hull (`mesh_hull_overshoot`).** Both are native-only settings, on by
default. The flat base assumes the object stands on the turntable; it is now
cut only when the silhouette hull below the support height is the shallow
cone a support plane leaves (no camera sees through the plane, so the hull
reaches below it only by about tan(elevation) times the footprint radius).
Floating means deeper than 15 % of the object's height and more than 1.15
times the cone the cameras' elevation explains (cameras looking down steeply,
as for YCB at about 52 degrees, make a supported box's cone deeper than 15 % of
its height, but not deeper than that cone). The four objects:
2.7 to 5.5 % of their height, support kept; the demo sphere from a 10-degree
ring and the asymmetric test object from a 20-degree ring: 26 %, no support,
no cut. (The cone itself cannot be the test: under a floating sphere the
hull is a cone of the same slope. A wide, flat object whose cone is deeper
than 15 % of its height keeps the hull cone under it instead of a flat
base.) Where nothing is
measured, the surface used to lie on the silhouette hull, which stands
outside the object by the mask tolerance and by what a ring of views cannot
carve. It is now moved inside by the hull's measured overshoot: the median
depth below the hull boundary of the measured zero-crossings, 0.7 voxels on
the demo sphere and 1.4 to 1.6 on the four objects. Demo sphere (24 views of
128 px, grid 96), radius by latitude band (true 1):

| surface | rms abs(r−1) | z min | z<−0.8 | −0.8..−0.4 | −0.4..0.4 | 0.4..0.8 | >0.8 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| hull | 0.0431 | −1.07 | 1.030±0.008 | 1.046±0.004 | 1.042±0.009 | 1.049±0.004 | 1.034±0.008 |
| before, final | 0.0696 | −0.67 | cut flat | 0.978±0.053 | 1.011±0.005 | 1.005±0.005 | 1.027±0.011 |
| after, level 0 preview | 0.0148 | −1.04 | 1.003±0.008 | 1.009±0.020 | 1.006±0.008 | 1.001±0.013 | 1.014±0.013 |
| after, final | 0.0132 | −1.04 | 1.006±0.008 | 1.013±0.014 | 1.011±0.005 | 1.005±0.005 | 1.013±0.009 |

On the four objects (photos, default providers) scanner F1 changes by +0.0013
to +0.0030 over the whole surface and by -0.0012 to +0.0014 above the
support; the tops (Bunny's back, Armadillo's shell, Lucy's wings and torch)
look the same on the sheets.

**Views that do not go around the object (`mesh_open_unseen`, native-only,
on).** On a capture from one side (DTU's robot arm: 49 views over about 100
degrees, no back views) the silhouette hull's unseen back stayed in the mesh
as a slab and dominated the accuracy. The stereo stage now marks the volume
`partial_views` when the unit vectors from the hull's centroid to the cameras
average to more than 0.6 (DTU 0.86 and 0.89; a ring at 10 to 35 degrees of
elevation stays below 0.6, so turntable captures are not affected; captures
that look down steeply, such as YCB at about 52 degrees, would be marked, by
the arithmetic, not yet run; their flat base is kept in the mesh). Surface extraction of such a volume orients the closed
largest component, then drops the triangles none of whose vertices touches a
voxel with measured or extrapolated evidence or the flat base; the mesh is
open there (`closed` false in `result.json`). Chosen against closing the
unseen part with a membrane (a harmonic field between the known voxels and
the outside of the hull, solved on a grid four times coarser), and against
also keeping hull surface that at least 3 or 8 cameras face. DTU protocol, mm
(accuracy / completeness / overall), supplied IDR masks:

| | closed (before) | open (adopted) | membrane | open plus faced hull |
| --- | --- | --- | --- | --- |
| scan 65, DTU poses | 1.906 / 0.949 / 1.428 | 1.549 / 0.925 / 1.237 | 1.403 / 1.166 / 1.285 | 1.553 / 0.929 / 1.241 |
| scan 63, DTU poses | 1.719 / 0.424 / 1.072 | 1.642 / 0.421 / 1.032 | 1.658 / 0.429 / 1.043 | |
| scan 65, `--cameras colmap --capture orbit` | 1.650 / 1.033 / 1.342 | 1.570 / 1.038 / 1.304 | | |
| scan 63, `--cameras colmap --capture orbit` | 1.909 / 0.874 / 1.392 | 1.744 / 0.874 / 1.309 | | |

These historical dense-stage diagnostics used supplied IDR masks, and the
`DTU poses` rows also used supplied camera poses. They do not establish
photo-only pipeline accuracy. Further pipeline validation must derive masks
and cameras from the photographs; dataset-supplied poses and masks, and
scanner geometry, are reserved for evaluation.

F1 at 1 mm rises with it (65 poses 0.688 to 0.714, 63 poses 0.812 to 0.820;
colmap 0.667 to 0.672 and 0.589 to 0.600). The membrane is the most accurate
on scan 65 but caves in the skull's white dome, which no view matched and
where the hull was right, so completeness suffers; the open mesh keeps it.
Full-turn captures are unchanged: Bunny and Dragon from their photos give
byte-identical meshes before and after.

**Handedness of the 3DLF scans.** Every reconstruction of the four test
objects fits its Revopoint scan only as a mirror image, with the AliceVision,
COLMAP and turntable cameras alike (Dragon, trimmed rms 0.60 mirrored against
1.98 proper). The reconstruction is not what is mirrored. `crisp3ds-dense
synthetic --capture --asymmetric` renders 48 photos of a sphere with three
bumps of different sizes on its +x, +y and +z axes, from the rig's camera
convention (x right, y down, z forward, world to camera). From those photos the
default command (threshold masks, turntable cameras) recovers cameras that
match the true ones by a proper similarity (rms 0.0027 against 0.545 for the
best mirrored one, orbit radius 6), and the mesh carried into the true frame
lies on the true object (median distance 0.0017 of the unit radius, 95 % within
0.021). The scan evaluator, given the exact object and its mirror image against
the true surface, scores the first proper and the second mirrored, and our
reconstruction proper (trimmed rms 0.0136 against 0.0140). So the 3DLF scans
are mirror images of the objects as photographed (a left-handed scan export,
or a mirrored frame in the scanner software); the evaluator says so in its
warning and scores the reflected fit, as before.

**Happy Buddha, diagnosed (no setting changed).** From its photos with the
default providers: F1 `all` 0.783 / 0.912 / 0.966, `above_margin` 0.821 /
0.944 / 0.984. Where the misses are (scan against mesh, height above the
plate): the lowest 5 % (closed underside and pedestal, 12 % of the mesh area,
69 % of it farther than 0.5 % of the diagonal), 5 to 25 % (pedestal and feet,
35 % far), the top quarter (raised arms and hands, 19 % far); the robe and
belly in between are 9 to 12 % far. As on the Dragon, an affine refit after
the evaluator's similarity finds axis scales 1.011 / 0.999 / 0.990 and lifts
F1 at 0.5 % from 0.773 to 0.788, and the fit is mirrored (rms 0.52 against
0.97), both properties of the scan. Depth: the finest level keeps 0.57 of the
mask with consistent depth (median over views) against 0.76 at 512 px; the
level fallback brings it to 0.75, so the soft face and robe are mostly
512-px depth. Tried, scores at 0.5 % `all` / `above_margin`: last window 13
(0.787 / 0.824) or 15 (0.786 / 0.823), coarser aggregation 1 / 1.5 / 3 (0.784
/ 0.822), finest tolerance 0.003 (0.784 / 0.823), two votes and best of two
(0.782 / 0.823), votes 3 / 3 / 2 (0.783 / 0.822). Window 13 on all five other
objects stays within 0.003 everywhere (Lucy +0.004 `all`, the others within
±0.0016) but looks the same on the sheets and changes a default shared with
the Python reference, so it is not adopted.

**GPU and CPU overlap in matching.** A readback is now split
(`Gpu::begin_read` queues the copy and submits it, `Gpu::finish_read` waits for
that submission only), and the matcher keeps one view in flight: view i's work
is queued and its readback started before view i-1's depth map is collected
and finished on the CPU (hull-front test, progress). Initial surfaces are
computed in groups of 16 with threads and one view at a time without (a
single-threaded browser), so each is computed while the GPU works on the
previous view. Results are byte-identical (Bunny: `depths.npz` and the mesh).
Bunny from its inputs, stereo stage: natively 44-49 s before, 41-44 s after
(two runs each, four threads); in Chrome 69.3 s before, 58.7 s after (last
pass 19.0 to 13.6 s), the same 1 045 002 triangles.

**Inspection sheets (`inspect/`).** Every run draws a sheet of each surface
it produces: the silhouette hull, the hull after mask repair, the surface after
each level (the live previews) and the final surface, each as three views
(the photo beside the surface rendered through that photo's camera) and one
detail crop per view, the square of the photo with the most fine detail inside
the mask (largest mean squared Laplacian), the same for every step.
`inspect/steps.png` puts one row per step under the photos. They are
announced as `inspection_sheet` artifacts with their `step`; the setting
`inspection_sheets` (native-only, on) turns them off; in a browser they are
skipped once WebAssembly memory reaches 2.5 GiB. The level rows show the
preview's coarseness; `--preview-step 1` draws them at full resolution.

**A level at the photos' native pixels (`native_level`, native-only, on).**
The finest level matched a canvas downscaled to 1024 px where the object's
canvas is larger. Now one more level at the canvas's own size follows (its
window, passes and tolerances those of the last level). Objects whose canvas
is at most 1024 px are unchanged (Bunny, Armadillo, Dragon: identical scores).
Scanner F1 at 0.5 / 1 / 2 %, before and after:

| | `all` before | `all` after | `above_margin` before | `above_margin` after |
| --- | --- | --- | --- | --- |
| Lucy (canvas about 1040 px) | 0.823 / 0.943 / 0.967 | 0.840 / 0.943 / 0.967 | 0.860 / 0.985 / 0.999 | 0.876 / 0.984 / 0.999 |
| Thai statue | 0.873 / 0.932 / 0.952 | 0.882 / 0.932 / 0.952 | 0.935 / 0.995 / 1.000 | 0.946 / 0.995 / 1.000 |
| Happy Buddha (1070 px) | 0.783 / 0.912 / 0.966 | 0.790 / 0.911 / 0.973 | 0.821 / 0.944 / 0.984 | 0.829 / 0.941 / 0.991 |

The largest drop anywhere is 0.0024 (Buddha `above_margin` at 1 %). Cost: one
more matching pass at native size (Buddha 98 s against 88 s).

**Sharpened matching images (`match_sharpening`, native-only, 0.6).** The
photos are soft (below), so the matching images get an unsharp mask (grey plus
0.6 times its difference from a one-pixel Gaussian blur) before NCC; the
masks, the fusion and the sheets use the photos as they are. On top of the
native level, F1 at 0.5 % `all` / `above_margin`: Lucy +0.007 / +0.007, Happy
Buddha +0.004 / +0.005, Dragon +0.003 / +0.004, Thai statue +0.001 / +0.001,
Armadillo +0.000 / +0.001, Bunny -0.002 / +0.002; the largest drop at any
threshold is 0.0026 (Bunny `all` at 1 %). Consistent coverage at the Buddha's
1024-px level rises from 0.569 to 0.582.

**Happy Buddha resolution study.** How much detail the photos carry inside the
object (every eighth photo; Laplacian variance, and the share of spectral
energy above a quarter and an eighth of the sampling frequency in a 128-px
patch at the object's centre):

| object | object box (px) | Laplacian variance | energy above fs/4 | above fs/8 |
| --- | --- | --- | --- | --- |
| Bunny | 685 x 812 | 2.8 | 0.009 | 0.030 |
| Armadillo | 578 x 764 | 5.9 | 0.005 | 0.011 |
| Dragon | 715 x 730 | 4.1 | 0.008 | 0.032 |
| Lucy | 460 x 980 | 4.6 | 0.002 | 0.008 |
| Thai statue | 494 x 958 | 6.6 | 0.004 | 0.016 |
| Happy Buddha | 405 x 992 | 4.0 | 0.012 | 0.034 |

All six are equally soft: 97 to 99 % of the energy lies below an eighth of the
sampling frequency, so the photos carry the detail of an image four times
smaller. The Buddha is not softer than the others; it is tall and thin with
fine relief (necklace beads, robe hems about 10 px wide), which a soft photo
and an 11-px matching window cannot resolve. What helped: matching once more
at native pixels (+0.007) and sharpening the matching images (+0.004 more);
an extra refinement pass at 1024 px with half the depth step (+0.001) and
windows 13 or 15 (+0.004, not adopted, see above) helped less. The closed
underside the scan does not have (6 % of the Buddha's mesh area) costs 0.025
of F1 `all` at 0.5 % and 1 % (0.773 to 0.798 on fresh samples): an evaluation
artefact; the flat base is cut where the support is and stays. A synthetic
capture (sphere with bumps, 960 x 720, 48 views) shows what sharper photos
would give: median distance to the true surface 0.0009 of the radius with
sharp photos, 0.0013 blurred by 1.5 px, 0.0038 by 3 px (95th percentile
0.013, 0.015, 0.031). The 3DLF photos sit between the last two by Laplacian
variance, so a camera with two to three times sharper optics would cut the
surface error by about half to two thirds; more pixels alone would not.

**Preset `detail` (`run --preset detail`, `RunOptions.preset`).** Searches 12
inverse-depth steps either side of the coarser surface at the finest levels
instead of 5. The diagnosis behind it: the necklace, belly lines and robe hems
of the Happy Buddha are in the photos but not in the finest depth maps, which
have holes exactly there; relief the coarser surface lacks lies outside the
narrow search band, the best score lands on its edge and the pixel is
rejected. Field smoothing is not the cause (Taubin off changes nothing; a
weaker field Gaussian adds noise, not detail, and costs the Armadillo 0.007),
nor are the variance gate (2 to 13 % of the crop pixels) or the score
threshold (0.45 instead of 0.55: +0.001). With the wider band the Buddha's
consistent coverage at native pixels rises from 0.53 to 0.66 and F1 at 0.5 %
by +0.011 whole and +0.017 above the support, but stray depth costs 0.006 at
2 %, so it is a choice, not the default. `describe()` lists the presets.

**Slanted planes per pixel at the finest level (`patchmatch`, research builds only).**
Normal native and WebAssembly builds exclude this stage and hide its controls.
Requests with `patchmatch=true` fail before reconstruction. The explicit Cargo
feature `research-patchmatch` retains the experiment; it does not grant patent
permission. A targeted [source and patent screen](../../tests/evidence/mvs-source-review.json)
identified claims requiring further review. Default band refinement is unchanged.
The measurements below describe earlier experimental builds.

 After the last band refinement each pixel carries a
plane (depth and normal in the camera's frame), scored with the matching
window sampled through the plane in every neighbour (`shaders/patchmatch.wgsl`:
the window's pixels are lifted to the plane along their own rays, projected
and sampled bilinearly as in `warp.wgsl`; NCC per neighbour, mean of the best
`best_of`; the window sampled at every second pixel). Planes start from the
band refinement where it found a depth and from the smooth surface of the
previous pass elsewhere, normals from that surface; then
`patchmatch_iterations` (4) red-black iterations, each taking for every pixel
the best of its plane, four planes of the other colour around it (per
direction the adjacent pixel or the one five pixels away, whichever scored
better, re-expressed at this pixel) and three random perturbations of depth
and normal whose size halves per iteration. Agreement between views then uses
the planes: a neighbour's depth at the exact projection comes from the plane
of the pixel it lands in, and its normal must agree to the cosine
`patchmatch_normal_agreement` (0.8). `stereo::patchmatch::reference` is the
scalar counterpart; on the analytic sphere the GPU scores equal it at every
pixel (to 1e-3), and four iterations from planes displaced by 3 % bring 99.7 %
of the pixels back within 1 % of the true depth.

Happy Buddha (F1 `all` / `above_margin` at 0.5 / 1 / 2 %, native-level
consistent coverage, stereo time of the native level on the shared M1):

| | `all` | `above_margin` | coverage | native level |
| --- | --- | --- | --- | --- |
| band refinement only (default) | 0.793 / 0.909 / 0.972 | 0.834 / 0.942 / 0.992 | 0.53 | about 20 s |
| planes, 8 iterations, eight candidates, depth votes | 0.830 / 0.925 / 0.964 | 0.866 / 0.954 / 0.984 | 0.70 | 305 s |
| planes, 4 iterations, four candidates, depth votes | 0.823 / 0.923 / 0.966 | 0.865 / 0.957 / 0.986 | 0.73 | 66 s |
| ... plane votes, no normal test | 0.825 / 0.925 / 0.966 | 0.866 / 0.958 / 0.986 | 0.73 | |
| ... plane votes, normals agree to 0.6 | 0.819 / 0.920 / 0.968 | 0.862 / 0.955 / 0.987 | 0.63 | |
| ... plane votes, normals agree to 0.8 | 0.817 / 0.918 / 0.971 | 0.859 / 0.952 / 0.990 | 0.53 | 68 s |
| ... plane votes, normals agree to 0.9 | 0.812 / 0.915 / 0.972 | 0.854 / 0.948 / 0.991 | 0.40 | |

Without the normal test the planes add stray surface (precision at 2 % 0.976
against 0.986 above the support); the normal test removes it at a cost in
coverage, and at 0.8 the Buddha stays within 0.002 of the default at 2 % while
gaining 0.024 at 0.5 %. On the sheets (`inspect/` detail crops: belly with
necklace, robe back, hem) the surface is smoother and less pitted; the
necklace beads and belly lines are still not resolved, so what limits them is
not only the search band.

Six objects with the settings above (`--set patchmatch=true`, everything
else default), F1 at 0.5 / 1 / 2 %, default then planes:

| | `all`, default | `all`, planes | `above_margin`, default | `above_margin`, planes | run, default / planes |
| --- | --- | --- | --- | --- | --- |
| Bunny | 0.903 / 0.937 / 0.954 | 0.907 / 0.939 / 0.956 | 0.965 / 0.995 / 1.000 | 0.967 / 0.995 / 1.000 | 165 / 138 s (loaded machine) |
| Armadillo | 0.927 / 0.974 / 0.990 | 0.934 / 0.973 / 0.989 | 0.952 / 0.996 / 1.000 | 0.960 / 0.997 / 1.000 | 60 / 112 s |
| Dragon | 0.806 / 0.933 / 0.982 | 0.810 / 0.937 / 0.982 | 0.852 / 0.966 / 0.995 | 0.856 / 0.969 / 0.995 | 154 / 135 s (loaded machine) |
| Lucy | 0.847 / 0.943 / 0.967 | 0.859 / 0.950 / 0.967 | 0.883 / 0.984 / 0.999 | 0.897 / 0.992 / 1.000 | 47 / 104 s |
| Thai statue | 0.882 / 0.932 / 0.952 | 0.892 / 0.934 / 0.952 | 0.947 / 0.995 / 1.000 | 0.959 / 0.998 / 1.000 | 79 / 136 s |
| Happy Buddha | 0.793 / 0.909 / 0.972 | 0.817 / 0.918 / 0.971 | 0.834 / 0.942 / 0.992 | 0.859 / 0.952 / 0.990 | 60 / 114 s |

No score drops by more than 0.002 (the adoption rule allows 0.003); every
object gains at 0.5 %. PatchMatch adds 47 to 69 s per run natively (73
views) in those measurements.

The five other objects' paired sheets were regenerated and reviewed on
2026-10-08, using one photo-derived camera/mask solution per object and
changing only `patchmatch`. Bunny, Armadillo and Dragon show less surface
noise, Lucy's robe folds are more continuous, and the Thai statue keeps a
similar silhouette. No obvious new structural loss was seen in the three
views per object, but small scales and engraving remain poorly resolved.
This was a visual review, not a rerun of the scanner scores above. The
review is recorded in [patchmatch-visual-review.json](../../tests/evidence/patchmatch-visual-review.json).
Photo-only DTU 65 and 63 validation is still required before a default change.

Browser check on 2026-10-08: Bunny from all 73 photos, threshold masks and
turntable cameras, Chrome 153 on the M1, four worker threads, the same
threaded WebAssembly build for both settings:

| | Full run | Stereo | Peak WebAssembly memory | Final surface |
| --- | --- | --- | --- | --- |
| Band refinement | 130.1 s | 59.7 s | 2.07 GiB | 1,040,494 triangles, closed |
| Slanted planes, 4 iterations | 275.8 s | 191.7 s | 1.82 GiB | 1,032,724 triangles, closed |

One run each, planes first; machine load and shader compilation can affect
the times. Memory is the WebAssembly linear-memory high-water mark, not
total browser or GPU memory; the lower plane-run value does not establish a
memory saving. The browser path works, but its measured extra 146 seconds
is a substantial cost. The setting remains off pending DTU validation.
Measurements are recorded in [patchmatch-browser.json](../../tests/evidence/patchmatch-browser.json).

**Bunny facial detail follow-up (2026-10-08).** Three close-up views of the
eyes, nose and cheeks show that the small global score gains above do not
restore convincing facial relief. Eight follow-up variations were rendered
through the same photo-derived cameras: almost no mesh smoothing, no coarse
depth fallback, a one-voxel fusion band, a smaller finest matching window with
less cost averaging, more separated neighbour cameras (band refinement and
slanted planes), a wider depth search with extra finest passes, and a 600
fusion grid instead of 400. None convincingly resolves the face. Removing
smoothing mainly exposes roughness (mesh genus 7 → 333); the finer grid
increases triangles from 1,032,764 to 2,422,948 with weak facial relief still.

The fusion experiments reuse the matched depths. The grid experiment rebuilds
the hull and mask repair at the new voxel size; smoothing and truncation still
use their configured voxel units, so this is not a comparison at identical
physical filter widths. The finest maps already contain holes and weak facial
relief before fusion. These checks rule out simply removing smoothing or
increasing mesh resolution as sufficient fixes; they do not isolate matching
from confidence filtering, residual camera error or limited photo information.
The recovered cameras have median / p95 sparse reprojection error 0.35 / 1.17
pixels, which is not a local facial depth-accuracy measurement.

Settings, mesh hashes, crop coordinates and qualitative observations are in
[bunny-head-detail-review.json](../../tests/evidence/bunny-head-detail-review.json).
No new scanner scores were measured and no defaults changed. Further detail
work needs local feature checks as well as whole-object F1: adaptive matching
support and geometric consistency are candidates, not validated fixes. The
comparison renderer now accepts per-view source-pixel `--crop` boxes, shared
by the photographs and freshly rasterised untextured meshes.

### Camera providers through the dense stages

A provider is judged by the reconstruction it leads to. `colmap` against
`alicevision`, same SAM masks (imported), native dense stages, scanner F1 at
0.5 % (1 %, 2 %). COLMAP ran through the PyCOLMAP 3.11 stand-in
(`tools/colmap_pycolmap.py`): default SIFT, exhaustive matching, masks on, one
shared `FULL_OPENCV` camera fixed to the declared lens.

| | Bunny | Armadillo | Dragon | Lucy |
| --- | --- | --- | --- | --- |
| Registered, `colmap` | 73 of 73 | 73 of 73 | 73 of 73 | 73 of 73 |
| Sparse points, `alicevision` / `colmap` | 9 087 / 2 813 | 6 677 / 5 240 | 5 619 / 3 360 | 3 266 / 2 143 |
| Reprojection median, p95 (px), `alicevision` | 0.42, 1.45 | not recorded | 0.52, 1.88 | 0.63, 2.31 |
| Reprojection median, p95 (px), `colmap` | 0.44, 1.92 | 0.42, 1.82 | 0.54, 2.16 | 0.61, 2.33 |
| Ring radius spread, `colmap` | 0.75 % | 0.44 % | 0.38 % | 0.79 % |
| Camera recovery (features, matching, SfM), `alicevision` | 1 021 s | not recorded | 494 s | 297 s |
| Camera recovery, `colmap` | 64 s | 105 s | 182 s | 133 s |
| F1 `all`, `alicevision` | 0.905 (0.941, 0.956) | 0.923 (0.971, 0.987) | 0.785 (0.928, 0.981) | 0.790 (0.904, 0.938) |
| F1 `all`, `colmap` | 0.905 (0.941, 0.956) | 0.924 (0.971, 0.987) | 0.795 (0.927, 0.982) | 0.828 (0.945, 0.968) |
| F1 `above_margin`, `alicevision` | 0.963 (0.995, 1.000) | 0.952 (0.996, 1.000) | 0.834 (0.963, 0.994) | 0.826 (0.945, 0.970) |
| F1 `above_margin`, `colmap` | 0.965 (0.996, 1.000) | 0.952 (0.996, 1.000) | 0.844 (0.961, 0.994) | 0.865 (0.986, 0.999) |

The AliceVision rows are the existing camera solutions with the native dense
stages on the existing inputs; their times are those of the Python stage's
runs (the Bunny's on a busier machine). COLMAP times were measured while
another job used the machine, with 4 threads against AliceVision's 2. One run
per object: neither program is repeatable, and the second Bunny run with
COLMAP that was planned was not made (the disk reached its floor). Not tried:
`--colmap-masks off`, sequential or ring matching, the COLMAP executable.

### The Bunny from its photos

`crisp3ds-dense photos` on the 73 Bunny photos, then the native dense stages
and the scanner evaluator. AliceVision 3.4 local build, 2 threads.

| | Python stage (run 220) | native, prefix called directly, `--masks threshold` | native, `av.py` wrapper, `--masks external-sam` |
| --- | --- | --- | --- |
| Masks | SAM 2.1 | Otsu threshold, median area 321 051 px | SAM 2.1 through the reference script: identical to the existing masks (0 of 147 M pixels) |
| Registered | 73 of 73 | 73 of 73 | 73 of 73 |
| Sparse points, observations | 8 638, 43 205 | 8 581, 42 690 | 8 788, 44 483 |
| Reprojection median, p95, max (px) | 0.416, 1.444, 18.6 | 0.414, 1.436, 17.7 | 0.423, 1.482, 28.6 |
| Ring radius spread, out of plane, largest gap | 0.48 %, 0.41 %, 6.08 deg | 0.48 %, 0.61 %, 6.12 deg | 0.41 %, 0.53 %, 6.07 deg |
| Duplicate pose pair reported | 13, 14 | 13, 14 | 13, 14 |
| Seconds: masks (coarse, SAM, cleanup, sheet) | 7.1, 81.8, 14.2, 4.6 | 2.2, none, 3.4, 1.8 | 1.9, 74.5, 3.4, 1.5 |
| Seconds: contrast images | 12.6 | 9.0 | 4.4 |
| Seconds: features, matching, global SfM | 152.8, 549.6, 318.4 | 262.5, 595.9, 318.4 | 104.6, 348.8, 266.5 |
| Seconds: audit; undistortion and scene; overlay | 2.0; 27.3; 2.0 | 0.02; 2.5; 0.5 | stopped before the audit |
| Gates | passed | passed | not reached: the run stopped at the free-space floor (7.4 GiB free, floor 8); the numbers above are from the reference audit of its `final.sfm` |
| F1 `all` at 0.5 / 1 / 2 % | 0.905 / 0.941 / 0.956 (cameras of run 218) | 0.896 / 0.929 / 0.947 | not run |
| F1 `above_margin` | 0.963 / 0.995 / 1.000 | 0.965 / 0.995 / 1.000 | not run |

The machine was shared with other jobs during both native runs, so the times
of the external steps say little; the steps done in this crate are 5 to 10
times faster than their Python counterparts. Command line of the middle
column:

```sh
crisp3ds-dense photos --photos <bunny>/rgb --calibration scripts/turntable_mesh/calibrations/3dlf-pro.json \
    --output <run>/frontend --events <run>/events.jsonl --threads 2 \
    --masks threshold --cameras alicevision \
    --alicevision .local-tools/alicevision-local/prefix --alicevision-library-path /opt/homebrew/lib
```

### Our own cameras: `--cameras turntable`

For one turn of ordered turntable photos with a known lens, from this crate
alone (`src/photos/turntable/`, `docs/TURNTABLE-SOLVER.md`): SIFT features
written here, mutual ratio matches between neighbouring photos around the
closed turn, one rotation about a fixed axis fitted to all consecutive pairs
as the starting point, tracks, and a bundle adjustment with free poses and the
lens fixed (Levenberg-Marquardt with a Schur complement). No crate added, no
random choice: two runs write identical `cameras.json` files. Builds for
wasm32.

Scanner F1 above the support at 0.5 %, same SAM masks, native dense stages:

| | Bunny | Armadillo | Dragon | Lucy |
| --- | --- | --- | --- | --- |
| `alicevision` | 0.963 | 0.952 | 0.834 | 0.826 |
| `colmap` | 0.965 | 0.952 | 0.844 | 0.865 |
| `turntable` | 0.966 | 0.953 | 0.837 | 0.860 |
| Camera recovery, `colmap` / `turntable` | 64 s / 18 s | 105 s / 24 s | 182 s / 18 s | 133 s / 13 s |

Every provider now also passes a closure gate: a capture is one closed turn
unless `--open-turn` says otherwise, and what is left from the last photo to
the first must be a step like the others.

`turntable` is the default camera provider and `threshold` the default mask
provider, so a run from photos starts no other program:

```sh
crisp3ds-dense run --photos data/bunny/rgb \
  --calibration scripts/turntable_mesh/calibrations/3dlf-pro.json \
  --output runs/bunny
```

One run per object, 73 photos, Apple M1 with 16 GB, scanner F1 at 0.5 %, 1 %
and 2 % of the scan's diagonal:

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

### Cameras from a printed mat: `--cameras markers`, `crisp3ds-dense mat`

`crisp3ds-dense mat --size a4|letter|a3 --output DIR` writes a mat of ArUco
markers (PDF, SVG, PNG, JSON); with it under the object,
`--cameras markers --markers-mat FILE.json` gets every pose from the markers:
no external program, repeatable, in millimetres, right-handed. Detection, pose
and the mat are in `src/photos/markers/` (plain Rust, builds for wasm32; no
crate added). `crisp3ds-dense mat --capture DIR` renders a synthetic capture
with known poses and `mat --compare` measures a scene against it.

Verified on rendered photos only; no printed mat has been photographed.
Measured there (`docs/MARKER-MAT.md` has the table): all photos registered in
24 captures from 5 to 60 degrees elevation, with blur up to 4 px, noise,
under- and overexposure and a large object; rotation error at most 0.09
degrees and position error at most 0.6 mm at 420 mm (0.004 degrees and
0.03 mm in the base capture); 808 markers found where OpenCV's ArUco detector
finds 800, corners 0.035 px from the truth in the median; the object
reconstructed by the dense stages 0.08 % smaller than its true 64 mm width.

### Rendered captures of textured meshes: `crisp3ds-dense render`

```sh
crisp3ds-dense render (--mesh MODEL.obj | --sphere) --output DIR [--views N (72)] [--elevation DEG (20)]
    [--width W (1749)] [--height H (1155)] [--fill F (0.7)] [--calibration LENS.json] [--distance D]
    [--samples 1|4 (4)] [--blur SIGMA_PX (0)] [--noise SIGMA (0)] [--backdrop GREY (0.86)] [--flat] [--threads N (4)]
```

A textured `.obj` (Google Scanned Objects load as they come, z up) on a light
disc in front of a backdrop, seen from one ring of cameras through the lens of
`--calibration` (its distortion included): every pixel's ray is undistorted
and cast against the mesh through a bounding volume hierarchy. One light above,
left of and in front of the camera turns with it, as a fixed light over a
turntable does; Lambert and ambient shading with a hard shadow. Writes
`photos/shot_NNN.png`, `masks/shot_NNN.png` (the exact silhouette, for
`--masks import:DIR` when the masks are not under test), `lens.json`,
`truth.json` (cameras, placement) and `reference.ply` (the mesh in the cameras'
frame, for `scan_evaluate --no-platform`). Evaluation on six GSO objects is in
`docs/OTHER-IMAGE-SETS.md`; the scripts that fetch, render, run, score and draw
the sheets are in `scripts/gso_eval/`.

### Tests

`cargo test` covers calibration scaling, envelope and threshold logic,
component labelling, cleanup rules, CLAHE and the colour conversions against
values written by OpenCV (`tests/fixtures/dense-native/opencv-contrast.json`,
produced by `make_photos_fixtures.py` next to it), the audit, ring statistics
and gate decisions (the cases of `test_photos_to_inputs.py`,
`test_silhouette_cleanup.py` and `test_alicevision_cameras.py`), the readers
for `.sfm` and COLMAP text and binary models, command lines of the three
external programs, bounded processes (deadline, cancel, process group), and the
whole stage on a synthetic capture with threshold masks and imported cameras.
`CRISP3DS_ALICEVISION_TESTS=1` with `CRISP3DS_ALICEVISION` (and, for a build
against Homebrew, `CRISP3DS_ALICEVISION_LIBRARY_PATH=/opt/homebrew/lib`) adds a
test that starts the real `cameraInit`.

## Inputs from a scene: `crisp3ds-dense inputs`

```sh
crisp3ds-dense inputs --scene final.sfm --prepared DIR --raw-masks DIR --output DIR [--self-contained [--link]]
crisp3ds-dense inputs --pack DIR [--output DIR] [--link]
```

An inputs directory is self-contained when its photos and masks lie inside it
and `cameras.json` names them by relative path; such a folder can be moved,
archived, or picked in a browser. The scene that `photos` writes is always
self-contained (`images/`, `masks/`). `inputs` by default is not: as in the
reference, `cameras.json` holds the absolute paths of the prepared images and
only the masks are inside. `--self-contained` copies the photos in as well;
`--pack DIR` does the same for an existing inputs directory, in place or as a
copy in `--output`; `--link` makes hard links instead of copies where the file
system allows (the Bunny's 146 files: linked in well under a second, 565 MB
that occupy no further space). `crisp3ds_dense::scene::pack` is the function.

Port of `dense_all_views_inputs.py`: one camera table for all registered views
of an AliceVision scene with one shared `radialk3` intrinsic, pointing at the
prepared images in place; raw photo masks remapped through the lens
undistortion with nearest sampling; the sparse points as `sparse_points.npy`.
The undistortion map follows OpenCV's `initUndistortRectifyMap` (double
precision, the camera matrix inverted by cofactors, coordinates accumulated
along each row, stored as float32) and `remap` with `INTER_NEAREST` (round half
to even, zero outside).

Parity with the Python module (OpenCV 4.10) on the Bunny and the Armadillo, 73
views each: every number of `cameras.json` identical, `sparse_points.npy`
identical, all 147 466 935 mask pixels identical. The translation is
accumulated with fused multiply-adds, which is what NumPy's matrix product does
on Apple silicon; a NumPy build that does not may differ in the last bit of a
translation.

## Photo check: `crisp3ds-dense check`

```sh
crisp3ds-dense check --inputs DIR --mesh mesh.stl --output DIR \
    [--repaired-masks DIR] [--preview-views N] [--check-views N] [--events events.jsonl]
```

Port of `mesh_photo_check.py` with the renderer of `stl_compare_render.py`:
silhouette intersection-over-union of the projected mesh against the input
masks and the repaired masks on evenly spaced views, `photo-overlay.png`,
`preview.png` (photos above, z-buffered flat-shaded mesh below), `result.json`
and the events. The silhouette is drawn as `cv2.fillPoly` draws a list of
triangles with four fractional bits: every edge as an 8-connected line between
the rounded vertex pixels, then an even-odd scanline fill of the whole edge
collection with span ends rounded to the nearest pixel.

Parity on the Bunny (mesh of 1 055 216 triangles, 26 views): the drawn
silhouettes are pixel-identical to OpenCV's on the views compared, and all IoU
numbers (median, minimum, maximum against input and repaired masks) equal the
Python module's to the last printed digit. The sheets show the same crops,
scales, colours and shading; they are resampled in float instead of OpenCV's
fixed point, and the two labels of the preview are set in a built-in 5x7 font
instead of OpenCV's Hershey font.

## Surface extraction: `crisp3ds-dense mesh`

Status: ported, at parity with `python -m scripts.turntable_mesh.tsdf_hull_mesh`
on the four test objects (table below). `dense_pipeline.py --native` calls it.

```sh
crisp3ds-dense mesh --volume stereo/volume.npz --output mesh \
    [--config config.json] [--set key=value ...] [--step N] \
    [--events events.jsonl] [--label TEXT] [--threads N]
```

Same behaviour as the Python module: refuses an existing output directory,
writes `mesh.stl` (binary, outward normals, largest component) and
`result.json` (same keys, same order), prints the report, and appends the
`final_mesh` artifact event with stage `mesh`, or `preview_mesh` with stage
`stereo` when `--step` is above 1. `--threads` (default 2) is the only addition.

| Step | Implementation (`src/mesh/`) |
| --- | --- |
| Hull prior | `edt.rs`: exact Euclidean distance transform, the separable lower-envelope algorithm of Felzenszwalb and Huttenlocher on integer squared distances; the square root equals `scipy.ndimage.distance_transform_edt` |
| Smoothing, extrapolation, final blur | `gaussian.rs`: separable, kernel cut at 4 sigma, `reflect` boundary, each line summed in double precision in SciPy's order and stored as `float32` between axes |
| Field | `field.rs`: the reference's arithmetic operation by operation, `float32` where NumPy works in `float32` |
| Iso-surface | `cubes.rs`: marching cubes whose cell boundaries are drawn face by face, ambiguous faces resolved by the asymptotic decider; one vertex per crossed grid edge |
| Mesh | `surface.rs`: largest component (union-find), Taubin smoothing (lambda 0.5, mu -0.53, uniform weights), edge counts, genus, signed volume |

Why the surface is a closed 2-manifold. The reference uses scikit-image's
Lewiner tables. Here a cell's triangles are derived from what the surface does
on the cell's six faces: marching-squares segments between the crossings on the
face's edges, where a face with two diagonally opposite inside corners is
decided by the sign of the bilinear interpolant at its saddle point. That
decision uses the four corner values of the face only, so the two cells sharing
a face draw the same segments. Each crossing ends exactly one segment on each
of its two faces, so the segments of a cell form closed rings; a ring is filled
with a fan whose diagonals never join two crossings on one face (a centre
vertex is added in the 140 of 656 table entries where no such fan exists, all
of them cells with an ambiguous face). Every mesh edge is therefore either
private to one cell and shared by two triangles of a fan, or a face segment
with exactly one triangle on either side. The tests check this exhaustively
for all 256 corner configurations and 64 face decisions, and on random fields.
Not ported: Lewiner's tests for tunnels through the interior of a cell, so the
genus can differ in a few cells (table).

Other deviations from the reference: sums are taken in a different order
(kernel normalisation, signed volume; the preview's signed volume is summed in
double instead of single precision); vertices are interpolated linearly without
scikit-image's `FLT_EPSILON` weights; among components with equally many
triangles the one with the lowest vertex index is kept; an empty hull or a
field without zero crossing is reported as an error; the event's keys are
written in alphabetical order and its path is relative only when the mesh lies
under the directory of the event log.

Parity on real volumes (hull grid about 400 voxels, re-fused from saved depths
with the default settings; Apple M-series, 2 threads; distances between the two
meshes on 200 000 area-weighted samples each way, in voxels; scanner scores
from `scan_evaluate.py` against the independent scans, which no stage reads):

| | Bunny | Armadillo | Dragon | Lucy |
| --- | --- | --- | --- | --- |
| Triangles, Python / native | 1 044 840 / 1 044 840 | 772 208 / 772 200 | 991 246 / 991 234 | 354 354 / 354 346 |
| Closed, both | yes | yes | yes | yes |
| Components (discarded faces), both | 8 (2416) | 10 (520) | 21 (4072) | 4 (124) |
| Genus, Python / native | 3 / 3 | 5 / 3 | 16 / 14 | 14 / 13 |
| Signed volume, relative difference | 1.6e-6 | 5e-7 | 6.1e-6 | 8.9e-6 |
| Distance Python to native: median / p99 / max | 0.003 / 0.049 / 0.29 | 0.005 / 0.065 / 0.26 | 0.006 / 0.076 / 0.56 | 0.005 / 0.069 / 0.60 |
| Distance native to Python: median / p99 / max | 0.003 / 0.048 / 0.26 | 0.005 / 0.065 / 0.27 | 0.006 / 0.075 / 0.32 | 0.005 / 0.068 / 0.37 |
| F1 `all` at 0.5 / 1 / 2 %, Python | 0.904 / 0.939 / 0.955 | 0.921 / 0.970 / 0.987 | 0.769 / 0.925 / 0.981 | 0.790 / 0.904 / 0.938 |
| F1 `all` at 0.5 / 1 / 2 %, native | 0.903 / 0.939 / 0.955 | 0.921 / 0.970 / 0.987 | 0.771 / 0.925 / 0.981 | 0.789 / 0.903 / 0.938 |
| F1 `above_margin`, Python | 0.964 / 0.995 / 1.000 | 0.951 / 0.996 / 1.000 | 0.820 / 0.960 / 0.994 | 0.827 / 0.945 / 0.970 |
| F1 `above_margin`, native | 0.963 / 0.995 / 1.000 | 0.951 / 0.996 / 1.000 | 0.822 / 0.961 / 0.994 | 0.826 / 0.945 / 0.970 |
| Largest F1 difference | 0.0007 | 0.0005 | 0.0018 | 0.0007 |
| Wall time, Python / native | 32.8 s / 7.4 s | 34.4 s / 7.5 s | 30.3 s / 8.5 s | 11.9 s / 3.3 s |
| Peak memory, Python / native | 3.3 GiB / 1.2 GiB | 3.0 GiB / 0.72 GiB | 2.4 GiB / 0.74 GiB | 1.0 GiB / 0.24 GiB |

`hull_fraction_below_support`, `observed_hull_fraction` and
`extrapolated_hull_fraction` are identical to all printed digits on the four
objects. Preview path (`--step 2`, Dragon): both closed, 245 442 / 245 436
triangles, genus 12 / 11, distance median 0.001, p99 0.19, maximum 1.05 voxels
(cells are two voxels wide there), 22.9 s / 6.2 s.

## Dense stereo: `crisp3ds-dense stereo`

Port of `python -m scripts.turntable_mesh.multiscale_stereo`: inputs, silhouette
hull, mask repair, coarse-to-fine matching, cross-view agreement and TSDF
fusion. `dense_pipeline.py --native` calls it.

```sh
crisp3ds-dense stereo --inputs DIR --output DIR [--config config.json] \
    [--set key=value ...] [--reuse-depths depths.npz] [--events events.jsonl] \
    [--previews] [--device NAME]
```

Same behaviour as the Python module: refuses an existing output directory;
writes `masks-repaired/`, `mask-repair.png`, `hull-vs-mask.png`,
`depth-level-N.png`, `depth-merged.png`, `depths.npz`, `volume.npz` and
`result.json`; with `--previews` writes `preview/*.npz` (renamed into place when
complete); appends the same progress, metric and artifact events. `--device` is
accepted and ignored (the adapter is chosen by `wgpu`); `result.json` carries
`device` (adapter and backend) and `engine` instead of `torch_version`, and a
few extra timings. `fused_passes` above 0 is refused (not ported; the default
is 0).

| Step | Where | Runs on |
| --- | --- | --- |
| Photos, masks, grey stretch, common canvas, neighbours | `inputs.rs` | CPU, 4 threads |
| Silhouette carving (`carve`, `build_hull`) | `hull.rs`, `shaders/carve.wgsl` | GPU over candidate voxel lists; grid geometry and candidate test on the CPU |
| Mask repair (`orbit_down`, `cover`, `repair_masks`) | `repair.rs`, `shaders/cover.wgsl` | GPU projection, CPU photo test |
| Pyramid levels (`build_level`) | `stereo/level.rs` | CPU; Pillow's BILINEAR and BOX reductions reimplemented |
| Matching (`score`, `ncc`, `_aggregate`, `_peak`, `sweep`, `refine`) | `stereo/matcher.rs`, `shaders/warp.wgsl`, `ncc_rows.wgsl`, `ncc_columns.wgsl`, `aggregate.wgsl`, `peak.wgsl` | GPU; the cost volume stays on the device |
| `initial`, `consistent`, `hull_front`, level fallback | `stereo/depth.rs` | CPU, float32, PyTorch's interpolation and pooling conventions |
| Fusion (`tsdf`) and support height | `fusion.rs`, `shaders/fuse.wgsl` | GPU over the hull's voxel list |
| Level loop, sheets, files, events | `stereo/levels.rs`, `stereo/run.rs`, `stereo/previews.rs` | CPU |

GPU use is plain WebGPU: WGSL compute shaders, storage and uniform buffers, no
features, the default limits. The device is requested with `Limits::default()`,
so a kernel that needs more than 8 storage buffers per stage, 128 MiB per
binding or 65 535 workgroups per dimension fails validation here as it would in
a browser. Work is chunked to fit: voxels in lists of 2 M, masks and depth maps
in batches of whole views below 112 MiB, hypotheses in chunks whose six row
sums per pixel fit one binding. The largest binding of a run is reported as
`gpu_peak_binding_bytes`. A level whose whole cost volume (hypotheses x pixels
x 4 bytes) exceeds 112 MiB is refused; with the default settings that is a
canvas above about 2.6 megapixels at the finest level. Raised limits are not
requested even where the adapter offers them.

Tests: `cargo test` runs everything that needs no GPU, including validation of
every shader with naga. `CRISP3DS_GPU_TESTS=1 cargo test` adds the kernels
against scalar implementations and the whole stage on an analytic sphere
(`stereo/synthetic.rs`, a port of `synthetic_scene.py`) with the thresholds of
`test_multiscale_stereo.py`. `CRISP3DS_GPU_FALLBACK=1` asks for a software
adapter.

Deliberate differences from the reference:

- Window sums of the matching score are accumulated row by row in float32
  instead of through `avg_pool2d`; bilinear samples, projections and votes are
  computed per element in WGSL rather than as tensor operations. Results differ
  in the last bits and, at thresholds and pixel boundaries, in single decisions.
- `orbit_down` takes the orbit normal from a double-precision eigenvector of
  the camera centres' scatter matrix instead of a single-precision SVD.
- `hull_front`, `initial`, `consistent` and the fallback merge run on the CPU.
- Preview sheets are resampled in float and rounded (Pillow resamples 8-bit
  data in fixed point); shading values above 1 saturate instead of wrapping.
- `result.json` keys are in alphabetical order.

Kept as in the reference although probably unintended there: all views of the
finer levels search with the inverse-depth step of the last view swept at the
coarsest level (`step` is overwritten per view in the level-0 loop). Giving
every view its own step was measured natively (`CRISP3DS_OWN_STEP=1`, SAM
masks, same cameras; scanner F1 at 0.5 % / 1 % / 2 %, own step minus shared
step):

| | `all`, shared step | `all`, own step | difference | `above_margin`, difference |
| --- | --- | --- | --- | --- |
| Bunny | 0.9049 / 0.9412 / 0.9560 | 0.9066 / 0.9419 / 0.9566 | +0.0017 / +0.0007 / +0.0006 | +0.0018 / +0.0005 / 0.0000 |
| Armadillo | 0.9226 / 0.9705 / 0.9867 | 0.9224 / 0.9708 / 0.9868 | -0.0002 / +0.0003 / +0.0001 | +0.0001 / +0.0003 / 0.0000 |
| Dragon | 0.7853 / 0.9284 / 0.9812 | 0.7760 / 0.9264 / 0.9809 | -0.0093 / -0.0020 / -0.0003 | -0.0110 / -0.0032 / -0.0009 |
| Lucy | 0.7899 / 0.9041 / 0.9379 | 0.7991 / 0.9064 / 0.9377 | +0.0092 / +0.0023 / -0.0002 | +0.0104 / +0.0027 / 0.0000 |

The Dragon loses about as much as Lucy gains, both only at the tightest
threshold, so the fix is not a general improvement and the shared step stays
the default (the rule was: keep it only if no score drops by more than 0.002).
The switch remains as an environment variable for further work; one run per
object, and the stage is deterministic on one machine.

Parity with the Python stage (Torch 2.7 on MPS, Apple M1; native on the same
GPU through Metal; default settings unless noted; 73 or 71 photos of
1749 x 1155).

Inputs and hull. Grey images, canvas boxes, neighbour lists and pyramid levels
(grey, mask, camera; three sizes) are bit-identical on the Bunny in all 73
views. Synthetic sphere: identical hull voxel set. Bunny without mask repair:
12 826 963 voxels on both sides, 7 only in Python and 7 only in native, each
with a projection within 0.0003 pixels of a rounding boundary. Depth bounds per
view then differ by up to 3.6e-4 relative, because the reference takes them
from every seventh hull voxel and the lists are shifted against each other;
with identical hulls they agree to float32 rounding.

Mask repair and fusion from the same saved depths (`--reuse-depths`):

| | Bunny | Armadillo | Dragon | Lucy |
| --- | --- | --- | --- | --- |
| Repaired mask pixels that differ (of 147 M; pixels added by repair) | 19 (714 488) | 15 (731 575) | 187 (779 416) | 94 (509 033) |
| Hull voxels, Python / native | 13 560 339 / 13 560 383 | 5 628 084 / 5 628 109 | 6 785 506 / 6 785 672 | 1 794 135 / 1 794 147 |
| Voxels only in Python / only in native | 7 / 51 | 4 / 29 | 7 / 173 | 6 / 18 |
| `weight` identical on common voxels; largest difference | 99.992 %; 2 | 99.984 %; 1 | 99.979 %; 3 | 99.972 %; 2 |
| `total` absolute difference, p99.9 / maximum | 0.00028 / 2.0 | 0.00037 / 1.7 | 0.00027 / 3.0 | 0.00037 / 2.0 |
| Largest F1 difference after the Python mesher | 0.0003 | 0.0001 | 0.0010 | 0.0003 |

With identical masks (Bunny, repair off) fusion alone gives: `weight` identical
in 99.993 % of voxels, largest difference 1 (one nearest-pixel lookup or one
truncation threshold decided the other way; the median distance of those
voxels' nearest lookup from a rounding boundary is 0.00009 pixels), `total`
p99.9 0.00025. Support height and observed fraction agree to all printed digits.

Whole stage, each side with its own matching:

| | Bunny | Armadillo | Dragon | Lucy |
| --- | --- | --- | --- | --- |
| Consistent coverage per pass, Python | 0.554 / 0.820 / 0.810 / 0.710 | 0.499 / 0.773 / 0.776 / 0.703 | 0.633 / 0.753 / 0.765 / 0.648 | 0.600 / 0.794 / 0.734 / 0.585 |
| Consistent coverage per pass, native | 0.556 / 0.821 / 0.809 / 0.711 | 0.498 / 0.772 / 0.776 / 0.701 | 0.634 / 0.752 / 0.764 / 0.648 | 0.598 / 0.794 / 0.735 / 0.589 |
| Largest coverage difference | 0.0013 | 0.0019 | 0.0012 | 0.0042 |
| Hull-front pixels, Python / native | 933 637 / 933 652 | 924 985 / 918 072 | 790 169 / 787 561 | 283 605 / 286 843 |
| Final depth valid in both, of valid in Python (median over views) | 97.3 % | 95.4 % | 96.1 % | 95.6 % |
| Relative depth difference where both valid, median / p95 | 5.0e-5 / 1.0e-3 | 1.1e-4 / 1.4e-3 | 6.3e-5 / 1.5e-3 | 8.0e-5 / 1.4e-3 |
| F1 `all` at 0.5 / 1 / 2 %, Python | 0.905 / 0.941 / 0.955 | 0.922 / 0.971 / 0.987 | 0.772 / 0.926 / 0.981 | 0.789 / 0.903 / 0.938 |
| F1 `all` at 0.5 / 1 / 2 %, native | 0.905 / 0.941 / 0.956 | 0.923 / 0.971 / 0.987 | 0.770 / 0.927 / 0.981 | 0.789 / 0.904 / 0.938 |
| Largest F1 difference (`all` and `above_margin`) | 0.0007 | 0.0008 | 0.0020 | 0.0005 |
| Largest GPU binding | 110.6 MiB | 110.9 MiB | 111.3 MiB | 104.3 MiB |
| Stage time, Python / native | 588 s / 78 s | 475 s / 58 s | 505 s / 66 s | 306 s / 40 s |

On the synthetic sphere, where the hulls are identical, the two stages agree
far more closely: coverage per pass within 0.0001, depth valid in both for at
least 99.9 % of each view's pixels, relative depth difference median 9e-8 and
p99 4e-6. On real objects the hulls differ in a few dozen voxels, so depth
bounds and with them the sweep planes are not the same.

Time per step on the Bunny, seconds, Python / native: hull
33.5 / 1.6; two repair rounds with their hulls 127 / 8.3; level 256 sweep
79.5 / 6.5; level 512 first pass with hull-front candidate 135.9 / 14.7;
level 512 second pass 41.6 / 5.2; level 876 121.9 / 17.8 (of which 4.5 on the
CPU for the initial surfaces and 0.9 for agreement); fusion 20.8 / 0.9;
everything else (loading, sheets, writing the files) 27.8 / 23.2, of which the
native stage spent 0.8 on loading and 21.8 on writing `volume.npz` and
`depths.npz` with single-threaded deflate. Members are now deflated in 4 MiB
pieces on four threads (same level, files a few bytes per piece larger, read by
NumPy and any inflater); writing took 12.8 s in a later run on a machine loaded
by other jobs. Storing without compression was not chosen: `depths.npz` and
`volume.npz` would grow from about 150 MB to about 480 MB on the Bunny, and the
depth file is kept with every run.

A later native run of the whole pipeline on the Bunny without live previews
(load average of the shared machine 6 to 7) took 51.8 s: loading 0.5, hull 1.1,
repair with its hulls 6.0, level 256 4.5, level 512 first pass 10.0, second pass
3.1, level 876 10.3, fusion 0.8, writing the files 7.3, the rest of the stereo
stage (sheets) 0.8, surface 4.8, check 2.4.

## Portability

Measured parity is from Metal on an Apple M1. Beyond that, the CI workflow runs
the six GPU tests (every kernel against its scalar implementation, the stereo
stage and the whole `run` on the analytic sphere, including cancellation) on
two software WebGPU adapters, with the same default limits: Mesa llvmpipe
through Vulkan on Linux (12 s) and the Microsoft Basic Render Driver (WARP)
through DirectX 12 on Windows (62 s). Both pass. The jobs are marked
non-blocking because software rasterisers on shared runners can be missing or
slow. No real Vulkan or DirectX GPU has run the kernels yet, and no real object
has been reconstructed natively on anything but Metal (in a browser: Chromium's
WebGPU on the same Mac, next section).

The whole crate, the photos stage included, is formatted, linted and tested
(`cargo test --features capi`) on Linux, macOS and Windows in the same
workflow, and each of the three runs the command-line smoke tests. A further
job installs the distribution's COLMAP on Ubuntu and runs `run --photos ...
--cameras colmap` on a synthetic capture (`docs/PHOTOS-TO-INPUTS.md` has the
numbers); it is non-blocking as well, for the same reason and because the
package may change under it.

The same code runs in a browser; see the next section.

## In the browser: `crates/dense/web`

The whole pipeline from photos (the photos stage with `threshold` masks and
`turntable` cameras, then `stereo`, `mesh`, `check`), or the dense stages from
an inputs directory, as WebAssembly with the kernels on WebGPU. Nothing in the library blocks or needs a
disk there:

- **Storage** (`src/storage.rs`). Every stage reads and writes through one
  module. A path whose first component is `mem:` names a file in an in-memory
  tree on any platform; in a browser every path does. The layout of a run
  directory and the events are the same. A host can also register a source that
  hands over files it keeps elsewhere, and a lister for the folders among them
  (the browser package keeps photos and inputs in JavaScript memory and hands
  them over one file at a time).
- **GPU** (`src/gpu/`). Device creation, kernel compilation, dispatch and
  readback are `async`. Native callers drive them with a blocking executor
  (`run`, `stereo::run::run`); a browser awaits `run_async`. In a browser,
  errors of dispatches are collected by the device's error callback and
  reported at the next readback, because awaiting an error scope per dispatch
  costs a round trip through the event loop each time.
- **Threads and time.** On wasm32 the CPU passes are plain loops, preview
  volumes are meshed on the spot instead of on a second thread, and time comes
  from `web-time`. The thread returns to its event loop at every readback, once
  per view while matching, so events are delivered and a cancel takes effect
  there; the CPU passes in between do not yield. Run the engine in a worker.

Build and test (the native build has no wasm dependencies; the bindings are a
crate of their own):

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129 --locked
sh crates/dense/web/build.sh                       # writes web/pkg (2.5 MB of WebAssembly)
cargo run --release -- synthetic --output web/scene # from crates/dense: a small test scene
cd crates/dense/web && npm install && npx playwright install chromium
node test/run.mjs --scene scene --output browser-run \
    --options '{"overrides":["sizes=64,128","grid=96","planes=48","neighbours=4","best_of=2","vote_neighbours=4","min_votes=2,2","crop_padding=6","hull_dilate=1","windows=5,7","aggregates=1,1"]}'
```

`index.html` with `main.js` and `worker.js` is a test page without a framework:
it fetches `files.json` and the files it lists, runs them in a worker and shows
status text and the final triangle count. `test/run.mjs` serves an inputs
directory to that page in headless Chromium and writes the events, the kept
output files, the adapter, the limits and memory figures; `--software` asks
Chromium for its software adapter (SwiftShader). `--photos DIR --calibration
FILE` starts from photos instead; `--channel chrome` uses an installed Google
Chrome instead of Playwright's Chromium (no per-process memory figures then).

```js
import { createRun, settingsSchema } from "./crisp3ds-dense.js";
const run = await createRun({
  files,                                  // Map: path in the inputs directory -> Uint8Array
  options: { settings: { grid: 320 } },   // fields of RunOptions; output and inputs are filled in
  onEvent: (event) => { /* the objects of events.jsonl */ },
});
const report = await run.finished;        // pipeline.json; rejects on failure or cancel
const stl = run.file("mesh/mesh.stl");    // any file of the run directory, e.g. an artifact's path
run.cancel(); run.dispose();

// From turntable photos: masks and cameras are recovered in the browser first.
const fromPhotos = await createRun({
  photos,                                 // Map or object: file name -> Uint8Array (PNG or JPEG), one turn in name order
  calibration,                            // crisp3ds_lens_calibration_v1 as an object, JSON text or bytes
  options: { live_previews: false, photo_options: [] },  // RunOptions; photo_options: words of `crisp3ds-dense photos`
  onEvent,
});
```

Only providers without external programs run in a browser: `--masks
threshold` or `import:DIR`, `--cameras turntable`, `markers` or `import:PATH`
(the defaults are `threshold` and `turntable`). The photos stay in JavaScript
memory until `dispose()` and are handed over one at a time; once the cameras
stage has written the scene, its files move out of WebAssembly memory to the
same place (`run.file("frontend/inputs/...")` still finds them). The events
add the `masks`, `cameras` and `inputs` stages in front of `stereo`.
`describe()` returns the start points of `run::describe()` for this build (in
a browser: photos with `threshold` or `import` masks and `turntable`,
`markers` or `import` cameras, and an inputs directory), for a form built from
data.

The bindings underneath (`web/src/lib.rs`): `Run` with `start(options, onEvent)`
and `cancel()`, `putFile`, `getFile`, `listFiles`, `removeTree`,
`setFileSource`, `setFileLister`, `describe`, `settingsSchema`,
`defaultSettings`, `version`.

Measured in headless Chromium 153 with WebGPU on an Apple M1. The adapter
reports vendor `apple`, architecture `metal-3`, not a fallback adapter. The
device is created with the default limits (128 MiB per storage binding, 256 MiB
per buffer, 8 storage buffers per stage); the largest binding was 110.6 MiB and
no limit had to be raised. Native numbers are from the same machine.

| | Synthetic sphere (small settings) | Bunny, 73 photos, default settings, no live previews |
| --- | --- | --- |
| Completes | yes | yes |
| Stage sequence, artifact kinds and counts, metric names against native | same | same |
| Triangles, native / browser | 33 792 / 33 792 | 1 047 200 / 1 047 356 |
| Closed, genus | yes, 0 | yes, 11 (native 11) |
| Consistent coverage per pass, largest difference | 0 | 0.0005 |
| Depth valid in both, of valid in either, per view: median / minimum | 100 % / 99.8 % | 99.6 % / 99.0 % |
| Relative depth difference where both valid: median / p95 / p99 | 0 / 4.7e-7 / 1.8e-6 | 2.9e-7 / 1.8e-5 / 2.6e-4 |
| Silhouette IoU median, native / browser | 0.92617 / 0.92617 | 0.9667 / 0.9668 |
| Run time, native / browser | 1.7 s / 2.5 s | 52 s / 143 s |
| Peak WebAssembly memory | 47 MiB | 1921 MiB |
| Peak resident memory: renderer / GPU process | 277 / 198 MiB | 2649 / 841 MiB |

Scanner F1 of the browser's Bunny mesh (Python evaluator, evaluation only):
`above_margin` 0.963 / 0.995 / 1.000, the same as native; `all` 0.905 / 0.941 /
0.956. With live previews the Bunny also completes (186 s) but peaks at 2539 MiB
of WebAssembly memory, because each preview volume is written and meshed while
the matching data is held; for megapixel sets in a browser pass
`live_previews: false`. Where the browser's time goes on the Bunny (seconds,
native / browser): hull 1.1 / 3.8, repair with its hulls 6.0 / 13.1, the four
matching passes 4.5, 10.0, 3.1, 10.3 / 7.4, 23.0, 8.6, 36.5 (of the last, 23.5
are the single-threaded initial surfaces), writing the compressed files 7.3 /
19.8, surface 4.8 / 12.3, check 2.4 / 7.7. The synthetic scene also completes on
Chromium's software adapter (vendor `google`, architecture `swiftshader`,
fallback) in 7.9 s with the same mesh.

From its 73 photos (100 MB of PNG) the Bunny completes in an installed Google
Chrome on the same M1 (headless, WebGPU on Metal, no live previews): 351 s in
all, against 93 s natively with four threads. Stages: masks 23 s, cameras
151 s (features 57 s and matching 55 s, both single-threaded; solve 10 s;
contrast and scene 25 s), stereo 139 s, mesh 23 s, check 14 s. All 73 photos
register; the cameras equal the native ones to 5e-11; 1 047 938 triangles,
closed, genus 7; scanner F1 `above_margin` 0.963 / 0.995 / 1.000 and `all`
0.902 / 0.938 / 0.955, as natively (0.964 / 0.995 / 1.000).

Peak WebAssembly memory of that run was 2499 MiB. What brought it to 2038 MiB
(each measured on the Bunny from photos):

| Change | Peak | Photos stage high-water |
| --- | --- | --- |
| Photos copied into the tree (first run) | 2499 MiB | 775 MiB |
| Photos handed over lazily (`setFileLister`) | 2404 MiB | 669 MiB |
| The scene moved out to JavaScript after the cameras stage | 2032 MiB | 672 MiB |
| Staged photos deleted once the contrast images exist (`reads_staged_photos`) | 2038 MiB | 570 MiB |

The photos stage now stays well below the dense stages: the peak was set in
`stereo`, 1468 MiB above where it starts, and 470 MiB of that came after
fusion, while `volume.npz` and `depths.npz` were encoded in memory. Now the
fused volume goes to the surface stage in memory inside one `run`
(`stereo::run::run_fused`, `mesh::run_volume`; the mesh is byte-identical to
the one made from `volume.npz`, checked on the Bunny), `volume.npz` is written
only with `keep_volume`, `depths.npz` is not written in a browser (natively it
is, for `--reuse-depths`), and the depth maps are released after fusion. The
Bunny from photos in Chrome, no live previews: peak 1568 MiB (from 2038),
nothing added after "Depth fused", 325 s, 1 045 330 triangles, closed, genus 7.
Live previews no longer write files either: the preview volumes go to the
preview mesher in memory (`mesh::field::Volume::hand_over`; the
`preview_volume` events name a path where nothing is written), and a coarse
preview is meshed on the volume coarsened by the preview step
(`mesh::run_preview`: blocks of step^3 voxels with summed evidence) instead of
on every step-th cell of the full grid, the same coarseness for an eighth of
the memory at step 2. The preview meshes change slightly (the Bunny's level-0
preview 252 996 triangles instead of 259 370, smoother, same shape); the final
mesh is byte-identical. Bunny from photos in Chrome with live previews: peak
1844 MiB (was 2685), four preview meshes, 470 s.

### Threads in the browser: a second package

`sh build.sh threads` writes `pkg-threads/`: the same crate with WebAssembly
threads (shared memory) and `--features threads`, which exports
`initThreadPool(n)` from `wasm-bindgen-rayon` (a rayon pool on Web Workers).
`crisp3ds-dense.js` loads it when the page is cross-origin isolated
(`crossOriginIsolated`) and the files are there, starts `min(cores, 4)`
workers (`load({ threads })`; `threads: 1` forces the default package), and
otherwise, or when the pool does not start within 30 s, loads `pkg/`.
`threadCount()` says which. `util::parallel` of the photos stage runs on the
pool in batches with progress between them; rayon's own passes (the surface
stage) use it as they are.

What it costs to build: stable rustc with `RUSTC_BOOTSTRAP=1`, the `rust-src`
component, `-Z build-std=panic_abort,std`, the target features `+atomics`,
`+bulk-memory`, `+mutable-globals` and linker arguments for shared, imported
memory of at most 4 GiB and the thread-local exports; a target directory of its
own (std and every crate compiled again, about 2.5 minutes here); and the
crates `wasm-bindgen-rayon` (with `no-bundler`, for pages without a bundler),
`wasm_sync` and `crossbeam-channel`. No nightly toolchain. wgpu compiled with
atomics without changes.

Where it runs: only cross-origin isolated. A server that can send
`Cross-Origin-Opener-Policy: same-origin` and
`Cross-Origin-Embedder-Policy: require-corp` does that directly
(`test/run.mjs --isolated`). Where headers cannot be set (GitHub Pages),
`coi-worker.js`, loaded first by the page as a classic script, registers itself
as a service worker that adds the headers and reloads the page once (the
approach of coi-serviceworker, MIT, written here in a few lines; it uses
`credentialless` where the browser has it). Every cross-origin resource the
page loads must then allow embedding. Verified in headless Chrome: without
headers the test page reloads once and runs with 4 threads; with `--threads 1`
it runs single-threaded.

Files the host hands over lazily: a JavaScript callback can be called only on
the thread that registered it, so pool workers never call the file source.
Work that reads such files gets them from the calling thread instead:
`photos::util::parallel_with_input` reads each batch's photos before the pool
decodes them (the `coarse` step), and `inputs::hold_for_pool` copies the
files a `parallel_map` reads (views in `inputs::load`, the input sheet, the
check's masks and photos) into the tree for the duration and releases them.
Photos and the scene therefore stay in JavaScript memory with threads too.
Events emitted on a pool worker, if any, would not reach `onEvent`.

`inputs::parallel_map` runs on the pool in the threaded package: initial
surfaces, cross-view agreement, mask repair, hull dilation and bounds, the
check and `.npz` deflate. In a browser `Gpu` records dispatches into one
command encoder and submits it at the next readback, clear or write instead
of once per dispatch (12 732 submits in the Bunny's stereo stage before, 1 149
after); natively every dispatch is still submitted at once inside its error
scope, so native results are unchanged (Bunny from photos, before and after:
`cameras.json`, `sparse_points.npy`, `depths.npz` and `mesh.stl` byte for
byte identical, twice). An A/B of 1, 8, 32 and unbounded dispatches per submit,
two rounds each, gave stereo times between 84 and 112 s with no order among
them: on this machine the gain is below the noise of shared GPU and CPU.

Bunny from photos with these changes, the two packages back to back (load
average 8 to 33, so times are high and only roughly comparable):

| | Default package | Threaded package, 4 workers |
| --- | --- | --- |
| Whole run | 432 s | 264 s |
| Photos stage (features, matching) | 234 s (97 s, 64 s) | 129 s (34 s, 25 s) |
| Stereo | 160 s | 116 s |
| Mesh, check | 23 s, 15 s | 12 s, 7 s |
| Peak WebAssembly memory | 1566 MiB | 1801 MiB |
| Triangles | 1 045 330 | 1 045 330 |

The threaded peak was 2458 MiB while photos and scene stayed in the tree; the
remaining 235 MiB above the default package are the views worked on at once.

Measured on the Bunny from its 73 photos in Chrome (headless, WebGPU on
Metal), the two packages back to back while the machine was shared with
other work (load average 15 to 23 on 8 cores), so absolute times are high:

| | Default package | Threaded package, 4 workers |
| --- | --- | --- |
| Whole run | 635 s | 466 s |
| Photos stage | 285 s (features 104 s, matching 80 s) | 150 s (features 39 s, matching 28 s) |
| Stereo | 274 s | 270 s |
| Mesh | 55 s | 19 s |
| Peak WebAssembly memory | 2038 MiB | 2458 MiB (photos and scene in the tree) |
| Triangles | 1 045 330 | 1 045 330 |

Threads take 27 % off the whole run here, and stereo, single-threaded in both,
is now the larger part. On an idle machine the default package took 250 s.

### Where stereo's time goes in a browser

Profiled on the Bunny's scene (73 views), dense stages only, the threaded
package in headless Chrome with a performance trace (`test/run.mjs --trace`),
on a quiet machine (load average 2 to 6), against the native run of the same
inputs with four threads:

| | Native | Browser |
| --- | --- | --- |
| Dense stages | 51 s | 88 s |
| Stereo | 43.9 s | 73.5 s |
| Matching levels (sizes 256, 512 twice, 880) | 4.4, 13.3, 10.5 s | 6.5, 22.0, 20.8 s |
| Silhouette hull | 1.1 s | 3.8 s |
| Fusion | 0.7 s | 1.7 s |

The earlier 270 s for stereo were measured while other jobs held the machine
(load 15 to 40); the gap on a quiet machine is 1.7 times, not 6.

The trace shows the engine's worker thread busy for 50.9 s of the 73.5 s and
idle for 22.5 s, in 19 453 gaps, 19 329 of them shorter than 5 ms. The idle
time is the wait for the GPU before each view's depth readback: the GPU
process's own thread is busy for only 2.1 s, so the wait is GPU execution
that nothing on the CPU overlaps. Inside the gaps run 12 732 command-buffer
flushes and 12 037 IPC messages: one per `queue.submit`, and `Gpu::run`
submits once per dispatch. Under load (the same runs side by side), the CPU
passes between kernels take the rest: initial surfaces 38 s (native 4.3 s),
cross-view agreement 12 s (native 1.1 s), mask repair with its hulls 25 s
(native 6.1 s). They all go through `inputs::parallel_map`, which is a plain
loop on wasm32 even in the threaded package, so threads did not shorten
stereo at all (the threaded run's stereo took as long as the default one).

Proposals, largest expected gain first:

1. **`inputs::parallel_map` on rayon in the threaded package** (the same
   `cfg(target_feature = "atomics")` branch as `photos::util::parallel`). It
   covers initial surfaces, cross-view agreement, mask repair, hull dilation
   and bounds, `.npz` deflate, the check and the scene, about 25 to 30 s of
   the quiet browser run on one thread; with four workers about 18 to 22 s
   less. One function in `src/inputs.rs`; the closures must not read files
   the host hands over lazily (below).
2. **CPU and GPU in parallel across views.** Today each view is dispatched,
   read back and only then the next started, so the 22.5 s of GPU time and
   the CPU work add up. Submitting view i+1 before reading view i back (two
   sets of work buffers in the matcher), and computing the next group's
   initial surfaces while the GPU runs, hides most of the shorter of the two:
   up to 15 to 20 s. Larger change in `stereo/levels.rs` and
   `stereo/matcher.rs`; GPU memory for a second set of work buffers.
3. **One submit per view instead of one per dispatch.** `Gpu::run` creates a
   command encoder and submits for every dispatch; a matcher pass encodes
   tens of dispatches per view. Recording them into one encoder and
   submitting once per view cuts about 12 700 flushes to about 400. Expected
   a few seconds of the engine thread's busy time (each flush and its IPC
   costs well under a millisecond), and less latency before the GPU starts.
4. **Readbacks** are already one per view and pass (the depth map) plus one
   per chunk in the hull and fusion; batching them further gains little
   once 2 overlaps them with work.
5. **`.npz` encoding** after fusion is the 470 MiB memory peak; the
   in-memory hand-off of the volume to the surface stage (in progress)
   removes the encoding from the browser path.

**Pool workers and files handed over lazily.** A JavaScript callback can be
called only on the thread that registered it, so a pool worker cannot read a
photo kept in JavaScript, and the threaded package keeps photos and scene in
WebAssembly memory instead. The smallest change that lifts this: read inputs
on the calling thread and hand the bytes to the pool. In the photos stage only
the `coarse` step reads the original photos (later steps read files the stage
wrote itself, which are in the tree), so a variant
`parallel_with_input(count, threads, load, work, watch)` of
`photos::util::parallel`, where `load(index)` reads the bytes on the calling
thread for the next batch and `work(index, bytes)` decodes them on the pool,
used by `staging::step_coarse_with`, is enough for the photos; for the scene,
the dense stages already load every view once in `inputs::load` on the
calling thread, so with proposal 1 the closures of `parallel_map` must keep
working on what was loaded and not open files themselves (the check stage's
panels are the one place to look at).

What a production browser build still lacks:

- **Threads.** The default package is single-threaded. The threaded package
  (below) parallelises the photos stage and the surface stage, not stereo's
  CPU passes (initial surfaces, agreement, deflate).
- **Memory.** WebAssembly memory is limited to 4 GiB and never shrinks. The
  Bunny (73 photos of 2 megapixels, hull grid 400) peaks at 1.9 GiB without
  live previews and 2.5 GiB with them; before the inputs were left with the
  host, the grey images cropped to the canvas after mask repair and released
  before fusion, it was 2.9 and 3.2 GiB. Larger sets need streamed fusion
  output and views decoded on demand. A level whose cost volume exceeds 112 MiB
  is refused (see above), as natively.
- **Persistence.** Outputs live in memory until the host takes them; nothing is
  written to the origin-private file system, and a run cannot be resumed.
- **Photo front end.** Runs from photos work with the providers without
  external programs; SAM, COLMAP and AliceVision are not available.
- **Coverage.** One browser engine (Chromium), one GPU (Apple M1 through
  Metal) and SwiftShader; no Firefox or Safari, no Windows or Linux GPU.

## Dependencies

License: AGPL-3.0-only, like the rest of the repository. Dependencies and their
licenses are listed here as they are added.

| Crate | License | Use |
| --- | --- | --- |
| anyhow | MIT OR Apache-2.0 | errors |
| serde, serde_json | MIT OR Apache-2.0 | configuration and events |
| rayon | MIT OR Apache-2.0 | worker threads of the grid passes |
| miniz_oxide | MIT OR Zlib OR Apache-2.0 | deflate for `.npz` members |
| crc32fast | MIT OR Apache-2.0 | `.npz` member checksums |
| wgpu | MIT OR Apache-2.0 | WebGPU compute (Metal, Vulkan, DirectX 12, browsers) |
| bytemuck | Zlib OR Apache-2.0 OR MIT | plain-data casts for GPU buffers |
| pollster | Apache-2.0 OR MIT | blocking on `wgpu` futures |
| image (png, jpeg only) | MIT OR Apache-2.0 | photo and mask decoding, PNG writing |
| fs4 | MIT OR Apache-2.0 | free disk space before a run (not on wasm32) |
| web-time | MIT OR Apache-2.0 | clock that also works in a browser |
| wasm-bindgen-rayon 1.3 with `no-bundler` (browser package `web/`, feature `threads` only) | Apache-2.0 | rayon's pool on Web Workers for the threaded browser package; brings wasm_sync (MIT OR Apache-2.0) and crossbeam-channel (MIT OR Apache-2.0) |
| libc (Unix only) | MIT OR Apache-2.0 | signalling the process group of an external tool (`src/photos/process.rs`) |
| ort 2.0.0-rc.13 (feature `sam-onnx` only, not on wasm32) | MIT OR Apache-2.0 | binding to ONNX Runtime for the `sam` mask provider; loads the runtime library at run time (`load-dynamic`), links nothing at build time |
| ureq 3 with rustls, ring, webpki-roots (feature `sam-onnx` only, not on wasm32) | MIT OR Apache-2.0; rustls Apache-2.0 OR ISC OR MIT; ring Apache-2.0 AND ISC; webpki-roots CDLA-Permissive-2.0 | fetching the default SAM model on first use (`src/photos/sam/fetch.rs`) |
| libloading 0.9 (feature `sam-ggml` only, not on wasm32) | ISC | opening CrispEmbed's `libcrispembed-sam2` at run time (`src/photos/sam/ggml.rs`) |

Runtime and model of the `sam` provider, neither part of the build: ONNX
Runtime (MIT; the shared library given with `--sam-runtime` or
`ORT_DYLIB_PATH`) or CrispEmbed's `libcrispembed-sam2` (CrispEmbed MIT, with
ggml MIT; `--sam-runtime` or `CRISPEMBED_SAM2_LIB`), and SAM 2.1 Hiera-tiny
converted to ONNX or GGUF (Apache-2.0, `cstr/sam2.1-hiera-tiny-GGUF` for the
GGUF;
Copyright Meta Platforms, Inc. and affiliates; `huggingface.co/cstr/sam2.1-hiera-tiny-ONNX`,
written by `tools/sam2_export_onnx.py`).

**SAM masks and PyTorch MPS.** Masks from `--masks external-sam` with
`--sam-device mps` (the default) made before 2026-10-06 came from a partly
wrong encoder: PyTorch 2.7 on MPS computes the pooled query of SAM 2's Hiera
encoder wrongly (see `scripts/turntable_mesh/README.md`; `segment.py` now
works around it). Every score "with SAM masks" in this file was made with those
masks. The `sam` provider computes like PyTorch on the CPU (Bunny: 69 of 73
masks identical to the reference script on the CPU, lowest IoU 0.999996).

Browser package only (`web/`): wasm-bindgen, wasm-bindgen-futures and js-sys
(MIT OR Apache-2.0) for the bindings, console_error_panic_hook (Apache-2.0 OR
MIT) to show panics in the console; tools wasm-bindgen-cli (MIT OR Apache-2.0)
to generate the package and, for the test page only, Playwright (Apache-2.0)
with its Chromium build.

External programs, started as child processes and never linked: COLMAP
(BSD-3-Clause), AliceVision (MPL-2.0, parts derived from libmv, MIT), and for
`--masks external-sam` a Python interpreter with SAM 2.1 (Apache-2.0) and
PyTorch (BSD-3-Clause). Development aids that are not part of any build:
`tools/colmap_pycolmap.py` uses the PyCOLMAP wheel (BSD-3-Clause), the scanner
evaluator and the parity scripts use NumPy, SciPy (BSD-3-Clause), Pillow
(MIT-CMU) and OpenCV (Apache-2.0). The CI jobs install Mesa (MIT) for the
software Vulkan adapter and the Ubuntu `colmap` package.

Native depth diagnostics: `CRISP3DS_STAGE_DEPTHS=1` retains finest-level
`depths-pass-N-raw.npz`, `depths-pass-N-consistent.npz` and adjusted camera
intrinsics. Raw means after photometric score acceptance, before cross-view
rejection; these are the reconstruction's own inferred depths. The ordinary
`depths.npz` contains the final maps after optional coarse fallback. The switch
only writes diagnostics and does not change geometry.


Depth ambiguity experiment (2026-10-09): `peak_min_margin=0.02` rejects a band
matching result when another local NCC maximum, more than two depth samples
away, is within 0.02 of the winning score. A valley more than 0.02 below both
maxima must separate them. Broad single peaks and an absence of competitors
remain accepted; this is not a general depth-uncertainty estimate. The default
is zero (disabled). It runs in the existing GPU peak reduction without adding
a depth volume or copying the cost volume to the CPU; it applies to sweep and
band refinement, not the optional independent-plane search.

All six photo-derived object controls pass the numerical regression gate, with
paired renders reviewed. Drill strict whole-object reference F1 improves
0.386 to 0.409, but severe defects remain. Bunny's tight eye-region F1 changes
0.582 to 0.586, without convincing recovery of eyes/nose; mean local surface
error does not improve. See the [distinct-mode review](../../tests/evidence/distinct-depth-modes-review.json).
Scanner geometry and supplied poses were used only in separate posthoc
diagnostics. No default or patent-clearance claim follows from these results.

Noise-matched truncation (2026-10-10, default `truncation_noise=3`): before
fusion, every second pixel of every depth map is projected into its
`vote_neighbours`; the robust spread (1.4826 × median absolute difference to the
depth interpolated there, differences beyond 2 % excluded) gives that pixel's
noise, and the median over pixels the object's noise σ. The truncation becomes
max(`truncation_voxels`, 3σ) and `behind_voxels` at least that; `result.json`
records it under `depth_noise`. `truncation_noise=0` reproduces the fixed
truncation byte for byte. Measured σ in voxels on the default runs: Thai statue
0.95 (truncation stays 3), Lucy 1.10, Happy Buddha 1.16, Bunny 1.47, Dragon 1.58,
Armadillo 1.71, YCB drill 2.8–4.5. Six objects from their own default depths:
worst F1 change −0.0013 (Happy Buddha at 1 %), Bunny +0.005 and Armadillo +0.006
at 0.5 %; a fixed 12-voxel truncation instead fails the gate (Thai −0.059), and
k=4 fails on Dragon (−0.0033). Drill with production defaults, same depths:
F1 at 0.5/1/2 % 0.364/0.577/0.788 → 0.489/0.732/0.874, mean error −24 %.
Numbers and limits: [noise-matched-truncation-review.json](../../tests/evidence/noise-matched-truncation-review.json).

Facial-detail experiments (2026-10-08): `slanted_refine=true` runs deterministic
coordinate refinement independently at each pixel. `slanted_iterations` defaults
to 6 (range 1–16). It reuses our plane scorer but does not borrow neighboring
plane candidates or use random search. `fusion_interpolate=true` interpolates
inverse depth at subpixel projections only with four valid corners and a depth
spread within the fixed truncation distance; otherwise fusion retains its nearest
sample. Neither changes the TSDF truncation width. Both switches default to
**false**. Independent plane refinement and research PatchMatch cannot be
enabled together. No third-party algorithm source was incorporated.

On identical recovered Bunny inputs, eye/brow-region F1 at 0.1% of the scanner
object diagonal is 0.5794 for default band matching, 0.5931 with independent
planes, 0.5820 for band plus subpixel fusion, and 0.6011 with both experiments.
Earlier research PatchMatch scored 0.6094. The combined mean symmetric surface
distance falls about 6.3% versus band. Frozen prior alignment and source-image
crop cones are used; scanner geometry participates only in posthoc scoring.
These are local metrics, not the six-object default-adoption gate. Three head
views still show poor eyes and nose; this is a modest improvement, not restored
facial detail. The default-off control mesh is byte-identical. See
[bunny-independent-detail-review.json](../../tests/evidence/bunny-independent-detail-review.json)
for measurements, hashes, tests and limitations. The source/patent screen above
records technical distinctions, without claiming legal clearance.

Example opt-in trial on the pipeline's own recovered inputs:

```sh
crisp3ds-dense stereo --inputs RUN/frontend/inputs --output trial/stereo \
  --set slanted_refine=true --set fusion_interpolate=true
crisp3ds-dense mesh --volume trial/stereo/volume.npz --output trial/mesh
```

The independent refinement alone added about 88 s on the native M1 Bunny run.
Browser execution time/memory and the other five objects have not been measured
for these new options.


Photo-textured GLB export is available with `run --texture` or the standalone
`texture` command, using the reconstruction's own cameras and photographs.
It is opt-in and preserves geometry. See [textured meshes](../../docs/TEXTURED-MESH.md)
for CLI/API/Studio use, visibility checks, coverage, comparisons and limitations.


Quality diagnostics and optional texture controls (2026-10-08):
`texture --coherent --color-balance` offers bounded source-view coherence and
exposure correction without changing geometry. Both are opt-in; visible seams
remain. SAM can use `--sam-prompt-mask background` to seed prompts from RGB
backdrop contrast. Defaults remain unchanged. See
[texture limits](../../docs/TEXTURED-MESH.md) and
[reserved-track/local-relief diagnostics](../../scripts/photo_regression/README.md).


Supported coarse fallback (experimental, default off): `fallback_supported=true`
requires coarse gap fills to agree with finest accepted depth maps in other
views. Fine pixels remain unchanged at the merge; coarse fills cannot vote for
one another. The final entries in `tolerances` and `min_votes` set this policy,
even when native image resolution stops the pyramid early. Angular voters use
`vote_neighbours`, `minimum_angle` and `maximum_angle` as usual. The option has
no effect when `fallback_level=false`. It changes depth reliability rather than
adding smoothing or inferring missing detail. Native and browser engines share
the implementation; ordinary defaults remain unchanged.

```sh
crisp3ds-dense run --inputs recovered-inputs --output supported-fallback \
  --set fallback_supported=true --set peak_min_margin=0.02
```

The distinct-mode setting is a separate opt-in control; it is not required by
the fallback algorithm. See [supported fallback evidence](../../tests/evidence/supported-fallback-review.json)
for native/browser validation and remaining shape/coverage limitations.


Coherent band experiment (2026-10-09, default off): `coherent_band=true`
uses one candidate depth/normal surface for each photometric support patch in
all finer-level and hull-front band searches. The normal comes from the local
initial surface and is fixed during the depth search. This removes a reproduced
failure in which neighbouring prior depths move a correct centre depth. It
bypasses offset-indexed cost aggregation; the coarse full sweep, masks, recovered
cameras, consistency and fusion are unchanged. It requires `neighbours<=8` and
`best_of<=4`, and uses every patch pixel. No neighbouring-plane propagation or
new upstream algorithm code is included.

With `peak_min_margin=0.02` and `fallback_supported=true`, Bunny head and drill
metrics improve over the same two-setting baseline, but inspected geometry is
still inadequate. Faster sampling, wider matching angles and existing normal
refinement do not give a generally better drill result. Keep this option off
pending full adoption evidence; it is not a repaired-all-objects preset. See
[depth quality audit](../../docs/DEPTH-QUALITY-AUDIT.md) and
[replay evidence](../../tests/evidence/coherent-band-review.json).
