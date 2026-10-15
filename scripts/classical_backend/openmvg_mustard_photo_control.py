"""One-shot, photo-only OpenMVG v2.1 SfM control on sealed mustard TRAIN JPEGs.

Default invocation is read-only. The worker receives only byte-identical JPEGs.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import re
import shutil
import subprocess
import time

from scripts.classical_backend import openmvg_photo_control as core
from scripts.classical_backend import openmvg_sceaux_photo_control_v2 as cli

SOURCE = Path("/Volumes/backups/code/crispmesh-data/mustard-sfm-train-001")
OUTPUT = Path("/Volumes/backups/code/crispmesh-data/openmvg-mustard-photo-sfm-001")
NAMES_SHA = "a81d1647109db27991c39c5fe1a9ae308c04081f8ada49d8b0f5ab1c15245544"
REPORT_SHA = "bbcba624f6a51f82e0ebdebf6e7e4cda10804bc8df34cfd0d5b3242983a225d0"
STAGE_SECONDS = (120, 600, 900, 600, 900)


def jpeg_dimensions(path: Path) -> tuple[int, int]:
    result = subprocess.run(["/usr/bin/sips", "-g", "pixelWidth", "-g", "pixelHeight", str(path)],
                            capture_output=True, text=True, timeout=10, check=True)
    widths = re.findall(r"^\s*pixelWidth:\s*(\d+)\s*$", result.stdout, re.M)
    heights = re.findall(r"^\s*pixelHeight:\s*(\d+)\s*$", result.stdout, re.M)
    if len(widths) != 1 or len(heights) != 1:
        raise RuntimeError(f"could not read unique JPEG dimensions: {path.name}")
    return int(widths[0]), int(heights[0])


def verify_photos(source: Path = SOURCE) -> dict:
    names_file = source / "train-names.txt"
    report_file = source / "stage-report.json"
    for item, expected in ((names_file, NAMES_SHA), (report_file, REPORT_SHA)):
        if item.is_symlink() or not item.is_file() or core.sha(item) != expected:
            raise RuntimeError(f"sealed TRAIN metadata differs: {item.name}")
    names = names_file.read_text().splitlines()
    report = json.loads(report_file.read_text())
    photos = report.get("train_photo_sha256", {})
    if (len(names) != 48 or len(set(names)) != 48 or
            any(not re.fullmatch(r"NP3_\d{3}\.jpg", name) for name in names) or
            report.get("status") != "complete" or
            report.get("train_names_sha256") != NAMES_SHA or
            report.get("train_names") != names or set(photos) != set(names) or
            len(report.get("heldout_names_excluded", [])) != 12 or
            set(report["heldout_names_excluded"]) & set(names)):
        raise RuntimeError("sealed TRAIN 48-name contract differs")
    image_dir = source / "images"
    if image_dir.is_symlink() or not image_dir.is_dir() or \
            {item.name for item in image_dir.iterdir()} != set(names):
        raise RuntimeError("TRAIN image directory has unexpected files")
    sizes = {}
    for name in names:
        item = image_dir / name
        if item.is_symlink() or not item.is_file() or core.sha(item) != photos[name]:
            raise RuntimeError(f"TRAIN photo seal differs: {name}")
        if jpeg_dimensions(item) != (1280, 1024):
            raise RuntimeError(f"TRAIN JPEG dimensions differ from 1280x1024: {name}")
        sizes[name] = item.stat().st_size
    return {"train_names_sha256": NAMES_SHA, "stage_report_sha256": REPORT_SHA,
            "names": names, "photo_sha256": {name: photos[name] for name in names},
            "photo_bytes": sizes, "photo_dimensions": [1280, 1024],
            "total_photo_bytes": sum(sizes.values())}


def commands(output: Path, build: Path = core.BUILD) -> list[tuple[str, list[str], str]]:
    stages = cli.commands(output, build)
    name, listing, artifact = stages[0]
    focal = listing.index("-f") + 1
    if name != "listing" or listing[focal] != "3398.4":
        raise RuntimeError("inherited Sceaux listing contract changed")
    listing[focal] = "1536"  # 1.2 × the sealed 1280-pixel JPEG width
    stages[0] = name, listing, artifact
    return stages


def capacity(output: Path, prospective: bool = False) -> dict:
    return core.capacity(output, prospective)


def preflight(output: Path = OUTPUT, source: Path = SOURCE) -> dict:
    if output.exists() or output.is_symlink():
        raise RuntimeError("one-shot mustard output must be fresh")
    if not output.parent.is_dir() or output.parent.is_symlink() or \
            output.parent.resolve() != core.BUILD.parent.resolve():
        raise RuntimeError("mustard output must be a fresh external sibling")
    photos = verify_photos(source)
    binaries = core.verify_binaries()
    space = capacity(output, prospective=True)
    usage = cli.audit_cli_usage(commands(output))
    return {"schema": "openmvg_mustard_train48_photo_control_v1",
            "status": "read_only_preflight", "source_images": str(source / "images"),
            "output": str(output), "photos": photos, "binaries": binaries,
            "compiled_cli_usage": usage, "capacity": space,
            "limits": {"output_bytes": core.OUTPUT_CAP, "rss_kib": core.RSS_CAP_KIB,
                       "log_bytes_per_stage": core.LOG_CAP,
                       "wall_seconds_total": core.TOTAL_SECONDS,
                       "wall_seconds_per_stage": STAGE_SECONDS, "threads_max": 2},
            "input_scope": "48 sealed TRAIN JPEGs only; no masks, poses, board, scanner, mesh or held-out photos",
            "license_scope": "evaluation only; no shipping or GPL/AGPL clearance"}


def report_counts(report: Path) -> dict:
    if report.is_symlink() or not report.is_file() or report.stat().st_size > core.LOG_CAP:
        raise RuntimeError("missing or oversized SfM HTML report")
    html = report.read_text(errors="replace")
    counts = {}
    for key in ("views", "poses", "intrinsics", "tracks", "residuals"):
        values = re.findall(r"#" + key + r":\s*(\d+)\s*<br\s*/?>", html)
        if len(values) != 1:
            raise RuntimeError(f"SfM report lacks unique #{key}")
        counts[key] = int(values[0])
    if counts["views"] != 48 or counts["poses"] > 48:
        raise RuntimeError("mustard SfM report view/pose counts inconsistent")
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
        for name in checked["photos"]["names"]:
            src, dst = SOURCE / "images" / name, output / "images" / name
            shutil.copyfile(src, dst)
            if core.sha(dst) != checked["photos"]["photo_sha256"][name]:
                raise RuntimeError(f"staged TRAIN JPEG differs: {name}")
            capacity(output)
        if verify_photos() != checked["photos"]:
            raise RuntimeError("sealed TRAIN photos changed during staging")
        receipt["staged_photo_sha256"] = {name: core.sha(output / "images" / name)
                                           for name in checked["photos"]["names"]}
        core.save(output, receipt)
        total_start = time.monotonic()
        for index, (name, command, expected) in enumerate(commands(output)):
            record = {"name": name, "command": command, "status": "running"}
            receipt["stages"].append(record)
            core.save(output, receipt)
            try:
                record.update(core.run_stage(command, output / "logs" / f"{index + 1:02}-{name}.log",
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
                             if receipt["sfm_report"]["poses"] == 48
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
    parser.add_argument("--run-mustard", action="store_true", help="one-shot live TRAIN photo run; requires review")
    args = parser.parse_args()
    print(json.dumps(run() if args.run_mustard else preflight(), indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
