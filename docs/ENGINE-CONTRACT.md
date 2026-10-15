# Engine contract for front ends

One front end is meant to serve macOS, Windows, Linux, iOS, Android and the
web. Only desktop systems run the reconstruction; phones and browsers are
clients of a machine that does. This document is what a front end may rely on.
The reference implementation is `scripts/turntable_mesh/` (`dense_events.py`,
`dense_pipeline.py`, `engine_server.py`, `export_replay.py`).

## 1. A run is a directory

Everything about a run lives in one directory. A front end never needs
anything else.

```
<run>/
  events.jsonl          the event log (section 2)
  config.json           the resolved settings
  pipeline.json         stage commands, exit codes, timings, final status
  cancel                created by a client to ask the run to stop
  input-sheet.png
  stereo/  mask-repair.png  hull-vs-mask.png  depth-level-N.png  depth-merged.png
           preview/<name>/mesh.stl            coarse surfaces while matching runs
  mesh/    mesh.stl  result.json
  check/   photo-overlay.png  preview.png  result.json
  scan/    overlay.png  result.json           only when a reference scan was given
```

Three ways to reach it, all with the same content:

| Mode | Events | Files |
| --- | --- | --- |
| Local (desktop shell) | read `events.jsonl`, remember the line count | read from disk |
| HTTP (phone, browser, remote) | `GET /api/runs/<id>/events?since=N` | `GET /api/runs/<id>/files/<path>` |
| Replay (no engine) | fetch the bundle's `events.jsonl` once | fetch relative to the bundle |

A replay bundle is made with
`python -m scripts.turntable_mesh.export_replay --run <run> --output <bundle>`.
`tests/fixtures/dense-run-sphere/` is one (a synthetic sphere, 35 events, 6 MB).

## 2. Events

`events.jsonl` is append-only, one JSON object per line. A reader's position is
the number of lines consumed; over HTTP each event carries that line number as
`seq`. A last line without a newline is still being written and must be ignored.

Every event has `type`, `time` (Unix seconds) and `stage` (`masks`, `cameras`,
`inputs`, `stereo`, `mesh`, `check`, `texture`, `evaluate`, or null for run-level events).
`masks` and `cameras` appear only in runs started from plain photos
(`docs/PHOTOS-TO-INPUTS.md`).

| `type` | Other fields | Meaning |
| --- | --- | --- |
| `run_started` | `schema`, `configuration`, `device`, `inputs` | First event of every run. `schema` is `crispmesh_dense_events_v1` |
| `stage_started` | | A stage began |
| `progress` | `fraction` (0..1 within the stage), `message` | For a progress bar and a status line |
| `metric` | `name`, `value` | A number worth showing, e.g. `coverage_level_1`, `silhouette_iou_median` |
| `artifact` | `kind`, `path` (relative to the run), `label`, optional `level`, `triangles` | A file is complete and can be shown |
| `stage_finished` | `seconds` | |
| `error` | `message` | A stage failed; `run_finished` follows |
| `run_finished` | `status` (`complete`, `failed`, `cancelled`), `seconds` | Last event |

Artifact kinds, in the order they normally appear:

| `kind` | File | What it shows |
| --- | --- | --- |
| `mask_sheet` | PNG | Photos-to-inputs step: segmentation masks over the photos |
| `sparse_overlay` | PNG | Photos-to-inputs step: recovered sparse points drawn on photos |
| `input_sheet` | PNG | Evenly spaced photos with their mask outlines |
| `preview_volume` | NPZ | Internal. Ignore it; a `preview_mesh` follows. Inside a native `run` the volume is handed to the preview mesher in memory and no file is written at the path |
| `preview_mesh` | binary STL | Coarse surface so far: the silhouette hull, the hull after mask repair, then the surface after each pyramid level except the last |
| `inspection_sheet` | PNG | `inspect/NN-step.png` after every preview surface and the final surface: the same three views (photo beside the surface through that camera) and the same automatically chosen detail crop per view each time; field `step` (`00-hull`, `01-hull-repaired`, `1N-level-N`, `90-final`). Last, `inspect/steps.png` (`step: "steps"`): one row per step. Drawn when the setting `inspection_sheets` is on (default) and, in a browser, while WebAssembly memory is below 2.5 GiB; per-level rows need live previews, and level rows show the preview's coarseness (`preview_step`) |
| `mask_repair_sheet` | PNG | Pixels that multi-view repair added to the masks, in green |
| `hull_mask_sheet` | PNG | Hull against masks: red where a mask is not covered, blue where the hull lies outside a mask |
| `depth_sheet` | PNG | Per level: photo, depth, depth shading for three views. `level` says which; the last one is the merged final depth |
| `final_mesh` | binary STL | The result at full resolution (can be 50 MB) |
| `textured_mesh` | GLB | Optional `texture: true` stage: preserved geometry with UVs and embedded photo atlas; a download, separate from STL viewer meshes |
| `photo_overlay` | PNG | Mesh outline against masks: green both, red mask only, blue mesh only |
| `preview_render` | PNG | Photos above, shaded reconstruction below |
| `scan_overlay` | PNG | Distance to an independent scan, when one was given |
| `report` | JSON | `mesh/result.json` (triangles, closedness, genus), `check/result.json` (silhouette agreement) and, if evaluated, `scan/result.json` |

Rules a front end can rely on:

- Every run that wrote `run_started` also writes `run_finished`, including when
  it fails before its first stage (an `error` event with the reason precedes it).
  Only a killed driver process can leave a log without it.
- `HEAD` works wherever `GET` does, so file sizes can be probed.
- An artifact event is written only after its file is complete.
- Preview meshes are produced by a side process and may arrive later than the
  matching events around them; order them by `seq`, show the newest.
- All meshes are in one coordinate frame, so a viewer can swap them without
  moving the camera. That frame has no physical scale and its up direction is
  arbitrary; a viewer should frame the bounding box.
- Unknown event types, artifact kinds and fields must be ignored, not treated
  as errors. New ones will be added without a schema change.

## 3. HTTP engine

`python -m scripts.turntable_mesh.engine_server --runs <dir> --data <dir> [--static <built front end>]`

| Request | Answer |
| --- | --- |
| `GET /api/health` | `{"schema", "device", "can_start_runs"}` |
| `GET /api/settings` | `{"settings": [{"name", "group", "meaning", "kind", "default"}]}`; `kind` is `boolean`, `integer`, `number`, `integer_list` or `number_list`. Enough to generate a settings form |
| `GET /api/data?path=<relative>` | `{"path", "entries": [{"name", "directory", "inputs", "photos", "calibration"}]}`: the data directory. `inputs` marks a folder a run can start from, `photos` a folder with at least three images, `calibration` a lens calibration file |
| `GET /api/runs` | `{"runs": [{"id", "status", "started", "stage", "stage_fraction", "events"}]}`, newest first |
| `POST /api/runs` | Body below. `201 {"id"}` or `400 {"error"}` |
| `GET /api/runs/<id>/events?since=N` | `{"events": [...], "next": M}`; poll with `since=M`, about once a second |
| `POST /api/runs/<id>/cancel` | `{"id", "cancel_requested": true}`; the run ends with `run_finished` status `cancelled` |
| `GET /api/runs/<id>/files/<path>` | The file, or 404 |

Start body:

```json
{
  "name": "my dragon",
  "inputs": "dragon/inputs",
  "device": "mps",
  "settings": {"grid": 320, "sizes": [256, 512]},
  "reference": "dragon/scan.ply"
}
```

`inputs` may be replaced by `scene`, `prepared` and `raw_masks`, or by `photos`
(a folder of turntable photos) with `calibration` (a lens calibration JSON);
the run then starts with the `masks` and `cameras` stages, which need
AliceVision and SAM 2.1 on the engine's machine. All paths are
relative to the engine's `--data` directory and are refused if they leave it.
`settings` keys are those of `GET /api/settings`; invalid values give a 400
before anything starts. `reference` is optional and is used only after the
reconstruction, for scoring.

Security model: the server binds to localhost by default. Bound to any other
address it requires `--token`, and clients send `Authorization: Bearer <token>`.
There is no TLS and there are no accounts; put it behind a reverse proxy or a
VPN for anything beyond a trusted network. CORS is open so a development front
end on another port can reach it.

## 4. Not in the contract yet

- **Photo upload and capture.** A phone cannot yet send photos; runs start from
  data already on the engine's machine. Planned as `POST /api/uploads`.
- **Push transport.** Clients poll. Server-sent events may be added; polling
  will keep working.
- **Mesh formats.** Only binary STL. A compact preview format (for example GLB
  with fewer triangles) may be added as an additional artifact kind.
