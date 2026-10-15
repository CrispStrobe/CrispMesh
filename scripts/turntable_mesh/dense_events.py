"""Run event log: the contract between the dense pipeline and any front end.

A run directory contains ``events.jsonl``, appended to by every stage, one JSON
object per line. A reader's position is simply the number of lines it has
consumed, so the same file serves a local desktop shell (tail the file), an
HTTP client (``GET .../events?since=N``) and offline replay (read it all).
Paths inside events are relative to the run directory.

Event types and their fields (every event also has ``time`` and ``stage``):

  run_started      configuration, device, inputs
  stage_started    -
  progress         fraction (0..1 within the stage), message
  metric           name, value
  artifact         kind, path, label, plus optional level / views / triangles
  stage_finished   seconds
  run_finished     status ("complete" | "failed" | "cancelled"), seconds
  error            message

Artifact kinds: mask_sheet, sparse_overlay, input_sheet, mask_repair_sheet, hull_mask_sheet, depth_sheet,
preview_volume (internal; the driver turns it into preview_mesh), preview_mesh,
final_mesh, photo_overlay, preview_render, scan_overlay, report.

Standard library only, so every stage and every interpreter can import it.
"""

import json
import os
from pathlib import Path
import time

SCHEMA = "crispmesh_dense_events_v1"
STAGES = ("masks", "cameras", "inputs", "stereo", "mesh", "check", "evaluate")
ARTIFACT_KINDS = ("mask_sheet", "sparse_overlay", "input_sheet", "mask_repair_sheet", "hull_mask_sheet", "depth_sheet", "preview_volume",
                  "preview_mesh", "final_mesh", "photo_overlay", "preview_render", "scan_overlay", "report")


class EventLog:
    """Append-only writer. ``EventLog(None)`` discards everything."""

    def __init__(self, path, stage=None):
        self.path = Path(path) if path else None
        self.stage = stage
        self.root = self.path.parent if self.path else None

    def emit(self, event_type, /, **fields):
        if self.path is None:
            return
        record = {"type": event_type, "time": time.time(), "stage": fields.pop("stage", self.stage), **fields}
        line = json.dumps(record, separators=(",", ":")) + "\n"
        # One write call per line: appends from several stage processes never interleave.
        with open(self.path, "a", encoding="utf-8") as stream:
            stream.write(line)
            stream.flush()
            os.fsync(stream.fileno())

    def progress(self, fraction, message=""):
        self.emit("progress", fraction=round(float(min(max(fraction, 0), 1)), 4), message=message)

    def artifact(self, kind, path, label, **extra):
        if self.path is None:
            return
        if kind not in ARTIFACT_KINDS:
            raise ValueError("unknown artifact kind " + kind)
        relative = os.path.relpath(Path(path).resolve(), self.root.resolve()).replace(os.sep, "/")
        self.emit("artifact", kind=kind, path=relative, label=label, **extra)

    def metric(self, name, value):
        self.emit("metric", name=name, value=value)


def read(path, since=0):
    """Events from line ``since`` on, with their line number as ``seq``. Ignores a half-written last line."""
    path = Path(path)
    if not path.is_file():
        return []
    events = []
    with open(path, encoding="utf-8") as stream:
        for number, line in enumerate(stream):
            if number < since or not line.endswith("\n"):
                continue
            try:
                events.append({"seq": number, **json.loads(line)})
            except json.JSONDecodeError:
                break
    return events
