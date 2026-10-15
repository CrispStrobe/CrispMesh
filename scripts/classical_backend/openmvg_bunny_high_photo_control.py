"""One-shot, photo-only OpenMVG HIGH-feature sparse bunny control.

Default invocation is a read-only preflight. The live worker sees only the 73
sealed contrast PNGs; no scanner data, capture calibration, or supplied poses.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
import shutil
import signal
import struct
import subprocess
import sys
import time
try:
    import resource
except ImportError:
    resource = None

from scripts.classical_backend import openmvg_photo_control as core
from scripts.classical_backend import openmvg_sceaux_photo_control_v2 as cli

SOURCE = Path("build-opencv/bunny-gamma05-clahe2")
COMPARISON_INPUTS = Path("build-opencv/classical-bunny-contrast-001/inputs.json")
OUTPUT = Path("/Volumes/backups/code/crispmesh-data/openmvg-bunny-high-photo-sfm-001")
PREPARE_SHA = "eca0bfa60badd7fa5c9b5311e7f413a3f337159c158f856066086e44b0a90ecd"
COMPARISON_SHA = "cd32fda6fb5945640d3e2bf7a33d41ba7990e11ea8116f646d3b64fa37b33f52"
NAMES = tuple(f"frame_{index:04}.png" for index in range(73))
WIDTH, HEIGHT = 1749, 1155
FOCAL_PX = "2098.8"  # image-only 1.2 x width heuristic, not capture calibration
TOTAL_SECONDS = 7200
STAGE_SECONDS = (120, 1800, 1800, 1200, 1800)


def png_dimensions(path: Path) -> tuple[int, int]:
    with path.open("rb") as stream:
        header = stream.read(24)
    if header[:16] != b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR" or len(header) != 24:
        raise RuntimeError(f"invalid PNG header: {path.name}")
    return struct.unpack(">II", header[16:24])


def verify_photos(source: Path = SOURCE, comparison: Path = COMPARISON_INPUTS) -> dict:
    manifest_file = source / "prepare-manifest.json"
    for path, expected in ((manifest_file, PREPARE_SHA), (comparison, COMPARISON_SHA)):
        if path.is_symlink() or not path.is_file() or core.sha(path) != expected:
            raise RuntimeError(f"bunny input manifest seal differs: {path.name}")
    manifest = json.loads(manifest_file.read_text())
    rows = manifest.get("images", [])
    comparison_rows = json.loads(comparison.read_text())
    if (manifest.get("schema") != "mve_image_only_preprocess_v1" or
            manifest.get("profile") != "gamma05_clahe2" or
            manifest.get("status") != "succeeded" or
            not isinstance(rows, list) or len(rows) != 73 or
            not isinstance(comparison_rows, list) or len(comparison_rows) != 73 or
            [row.get("output") for row in rows] != list(NAMES) or
            [row.get("name") for row in comparison_rows] != list(NAMES) or
            {item.name for item in source.iterdir()} != {*NAMES, "prepare-manifest.json"}):
        raise RuntimeError("bunny 73-photo inventory or preprocess contract differs")
    source_names = [row.get("source") for row in rows]
    if set(source_names) != {f"bunny_{index}_rgb.png" for index in range(73)}:
        raise RuntimeError("bunny numeric source-frame mapping differs")
    hashes = {}
    total = 0
    for name, row, other in zip(NAMES, rows, comparison_rows):
        path = source / name
        if (path.is_symlink() or not path.is_file() or
                other.get("source") != str(path) or
                row.get("width") != WIDTH or row.get("height") != HEIGHT or
                png_dimensions(path) != (WIDTH, HEIGHT) or
                row.get("output_sha256") != other.get("sha256") or
                path.stat().st_size != other.get("bytes") or
                core.sha(path) != row.get("output_sha256")):
            raise RuntimeError(f"bunny input photo seal differs: {name}")
        hashes[name] = row["output_sha256"]
        total += other["bytes"]
    return {"prepare_manifest_sha256": PREPARE_SHA,
            "comparison_inputs_sha256": COMPARISON_SHA,
            "photo_sha256": hashes, "total_photo_bytes": total,
            "source_frame_by_photo": dict(zip(NAMES, source_names)),
            "dimensions": [WIDTH, HEIGHT], "count": len(NAMES)}


def commands(output: Path, build: Path = core.BUILD) -> list[tuple[str, list[str], str]]:
    stages = cli.commands(output, build)
    listing = stages[0][1]
    if listing[listing.index("-f") + 1] != "3398.4":
        raise RuntimeError("inherited focal command changed")
    listing[listing.index("-f") + 1] = FOCAL_PX
    feature = stages[1][1]
    if feature[feature.index("-p") + 1] != "NORMAL" or "-n" in feature:
        raise RuntimeError("inherited feature command changed")
    feature[feature.index("-p") + 1] = "HIGH"
    return stages


def preflight(output: Path = OUTPUT, source: Path = SOURCE,
              comparison: Path = COMPARISON_INPUTS) -> dict:
    if output.exists() or output.is_symlink():
        raise RuntimeError("one-shot bunny output must be fresh")
    if (not output.parent.is_dir() or output.parent.is_symlink() or
            output.parent.resolve() != core.BUILD.parent.resolve()):
        raise RuntimeError("bunny output must be a fresh external sibling")
    photos = verify_photos(source, comparison)
    binaries = core.verify_binaries()
    usage = cli.audit_cli_usage(commands(output))
    capacity = core.capacity(output, prospective=True)
    return {"schema": "openmvg_bunny_high_photo_control_v1",
            "status": "read_only_preflight", "source_images": str(source),
            "comparison_inputs": str(comparison), "output": str(output),
            "photos": photos, "binaries": binaries, "compiled_cli_usage": usage,
            "capacity": capacity,
            "limits": {"output_bytes": core.OUTPUT_CAP, "rss_kib": core.RSS_CAP_KIB,
                       "log_bytes_per_stage": core.LOG_CAP,
                       "wall_seconds_total": TOTAL_SECONDS,
                       "wall_seconds_per_stage": STAGE_SECONDS, "threads_max": 2,
                       "external_internal_floor_bytes": core.FLOOR + core.MARGIN},
            "initial_focal": {"pixels": float(FOCAL_PX), "method": "1.2 x 1749 image width heuristic"},
            "input_scope": "73 sealed contrast PNGs only; no masks, depth, scanner, capture calibration, or supplied poses",
            "dense_gate": "independent camera geometry audit required; pose count alone does not authorize dense",
            "license_scope": "evaluation only; no shipping or GPL/AGPL clearance"}


def run_stage(command: list[str], log: Path, output: Path, seconds: int,
              total_start: float) -> dict:
    if sys.platform != "darwin" or resource is None:
        raise RuntimeError("native execution requires macOS resource limits")
    started = time.monotonic()
    env = os.environ.copy()
    env.update({"OMP_NUM_THREADS": "2", "OPENBLAS_NUM_THREADS": "2",
                "VECLIB_MAXIMUM_THREADS": "2", "TMPDIR": str(output / "tmp"),
                "TMP": str(output / "tmp"), "TEMP": str(output / "tmp")})
    peak = 0

    def cpu_limit() -> None:
        resource.setrlimit(resource.RLIMIT_CPU, (2 * seconds, 2 * seconds + 1))

    with log.open("xb") as stream:
        proc = subprocess.Popen(command, stdout=stream, stderr=subprocess.STDOUT,
                                env=env, start_new_session=True, preexec_fn=cpu_limit)
        try:
            while proc.poll() is None:
                if (time.monotonic() - started > seconds or
                        time.monotonic() - total_start > TOTAL_SECONDS):
                    raise RuntimeError("stage or total wall cap exceeded")
                if log.stat().st_size > core.LOG_CAP:
                    raise RuntimeError("stage log cap exceeded")
                core.capacity(output)
                table = subprocess.check_output(["/bin/ps", "-axo", "pid=,ppid=,rss="], text=True)
                peak = max(peak, core.base.process_rss_kib(proc.pid, table))
                if peak > core.RSS_CAP_KIB:
                    raise RuntimeError("process-tree RSS cap exceeded")
                time.sleep(0.5)
            if proc.returncode or log.stat().st_size > core.LOG_CAP:
                raise RuntimeError(f"stage returned {proc.returncode} or log exceeded cap")
            core.capacity(output)
        except BaseException:
            if proc.poll() is None:
                os.killpg(proc.pid, signal.SIGTERM)
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(proc.pid, signal.SIGKILL)
                    proc.wait(timeout=5)
            raise
    return {"wall_seconds": round(time.monotonic() - started, 3),
            "peak_rss_kib": peak, "log_sha256": core.sha(log),
            "log_bytes": log.stat().st_size}


def report_counts(report: Path) -> dict:
    if report.is_symlink() or not report.is_file() or report.stat().st_size > core.LOG_CAP:
        raise RuntimeError("missing or oversized bunny SfM report")
    html = report.read_text(errors="replace")
    counts = {}
    for key in ("views", "poses", "intrinsics", "tracks", "residuals"):
        matches = re.findall(r"#" + key + r":\s*(\d+)\s*<br\s*/?>", html)
        if len(matches) != 1:
            raise RuntimeError(f"bunny SfM report lacks unique #{key}")
        counts[key] = int(matches[0])
    if counts["views"] != len(NAMES) or counts["poses"] > len(NAMES):
        raise RuntimeError("bunny SfM report view count inconsistent")
    return counts


def run(output: Path = OUTPUT) -> dict:
    checked = preflight(output)
    output.mkdir()
    for name in ("images", "matches", "sparse", "logs", "tmp"):
        (output / name).mkdir()
    receipt = {**checked, "status": "running", "started_utc": datetime.now(timezone.utc).isoformat(),
               "stages": []}
    core.save(output, receipt)
    try:
        for name in NAMES:
            target = output / "images" / name
            shutil.copyfile(SOURCE / name, target)
            if core.sha(target) != checked["photos"]["photo_sha256"][name]:
                raise RuntimeError(f"staged bunny photo differs: {name}")
            core.capacity(output)
        if verify_photos() != checked["photos"]:
            raise RuntimeError("source bunny photos changed during staging")
        receipt["staged_photo_sha256"] = {name: core.sha(output / "images" / name) for name in NAMES}
        core.save(output, receipt)
        total_start = time.monotonic()
        for index, (name, command, expected) in enumerate(commands(output)):
            record = {"name": name, "status": "running", "command": command}
            receipt["stages"].append(record)
            core.save(output, receipt)
            try:
                record.update(run_stage(command, output / "logs" / f"{index + 1:02}-{name}.log",
                                        output, STAGE_SECONDS[index], total_start))
                artifact = Path(expected)
                if artifact.is_symlink() or not artifact.is_file() or artifact.stat().st_size == 0:
                    raise RuntimeError(f"required stage artifact missing: {artifact}")
                record["artifact_sha256"] = core.sha(artifact)
                record["status"] = "completed"
            except BaseException as exc:
                record["status"] = "stopped"
                record["reason"] = str(exc)
                raise
            finally:
                record["output_bytes"] = core.base.tree_bytes(output)
                core.save(output, receipt)
        receipt["sfm_report"] = report_counts(output / "sparse/SfMReconstruction_Report.html")
        receipt["status"] = ("completed_pending_geometry_review"
                             if receipt["sfm_report"]["poses"] == len(NAMES)
                             and receipt["sfm_report"]["tracks"] > 0
                             and receipt["sfm_report"]["residuals"] > 0
                             else "failed_registration_or_sparse_gate")
    except BaseException as exc:
        receipt["status"] = "stopped"
        receipt["reason"] = str(exc)
        raise
    finally:
        receipt["finished_utc"] = datetime.now(timezone.utc).isoformat()
        receipt["output_inventory"] = core.output_inventory(output)
        core.save(output, receipt)
    return receipt


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run-bunny", action="store_true", help="one-shot live 73-photo SfM")
    args = parser.parse_args()
    print(json.dumps(run() if args.run_bunny else preflight(), indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
