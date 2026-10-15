"""Seal and stage the 73 contrast bunny PNGs for an Apple .preview trial.

Default invocation is read-only. --stage-input copies only images to a fresh
external directory. This module does not launch Object Capture.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import shutil

from scripts.classical_backend import openmvg_bunny_high_photo_control as bunny
from scripts.classical_backend.run import digest, folder_bytes


ROOT = Path("/Volumes/backups/code/crispmesh-data")
INPUT = ROOT / "apple-bunny73-preview-001-input"
OUTPUT = ROOT / "apple-bunny73-preview-001-output"
PROBE = Path(".local-tools/tmp/apple-probe.9SE5u9/photogrammetry-probe")
PROBE_SHA = "f6fe2bbb5677177eeba8b1b19b8a1c49c2e785305b9cfa1f781b67ae10bd277b"
FLOOR = 11 << 30
INPUT_CAP = 130 << 20
OUTPUT_CAP = 512 << 20
TOTAL_CAP = 650 << 20
SCHEMA = "apple_bunny73_preview_input_v1"


def preflight(input_dir: Path = INPUT, output_dir: Path = OUTPUT,
              source_dir: Path = bunny.SOURCE, probe: Path = PROBE) -> dict:
    if (input_dir.exists() or input_dir.is_symlink() or
            output_dir.exists() or output_dir.is_symlink()):
        raise ValueError("both external trial paths must be fresh")
    if (input_dir.parent.resolve() != ROOT.resolve() or
            output_dir.parent.resolve() != ROOT.resolve() or
            input_dir.parent.is_symlink() or output_dir.parent.is_symlink()):
        raise ValueError("trial input and output must be fresh external siblings")
    if input_dir == output_dir:
        raise ValueError("input and output paths must differ")
    if probe.is_symlink() or not probe.is_file() or digest(probe) != PROBE_SHA:
        raise ValueError("compiled Apple probe differs from sealed binary")
    photos = bunny.verify_photos(source_dir)
    if photos["count"] != 73 or photos["total_photo_bytes"] > INPUT_CAP:
        raise ValueError("processed bunny photo set exceeds input cap")
    # Reserve room for both the staged input and the whole bounded output.
    if photos["total_photo_bytes"] + OUTPUT_CAP + (1 << 20) > TOTAL_CAP:
        raise ValueError("input plus output would exceed 650 MiB total cap")
    external_free = shutil.disk_usage(ROOT).free
    internal_free = shutil.disk_usage(source_dir).free
    if external_free < FLOOR + TOTAL_CAP or internal_free < FLOOR:
        raise ValueError("11 GiB disk floor plus bounded external headroom unavailable")
    return {"schema": SCHEMA, "status": "read_only_preflight",
            "scope": "evaluation_only_apple_preview; image_only; no scanner or supplied poses",
            "source": str(source_dir.resolve()),
            "source_prepare_manifest_sha256": photos["prepare_manifest_sha256"],
            "source_comparison_inputs_sha256": photos["comparison_inputs_sha256"],
            "source_frame_by_photo": photos["source_frame_by_photo"],
            "source_photo_sha256": photos["photo_sha256"],
            "source_photo_bytes": photos["total_photo_bytes"],
            "input_dir": str(input_dir), "output_dir": str(output_dir),
            "probe": str(probe.resolve()), "probe_sha256": PROBE_SHA,
            "limits": {"staged_input_cap_bytes": INPUT_CAP,
                       "launcher_output_cap_bytes": OUTPUT_CAP,
                       "total_new_input_output_cap_bytes": TOTAL_CAP,
                       "min_free_bytes_each_disk": FLOOR,
                       "timeout_minutes": 15},
            "capacity": {"external_free_before": external_free,
                         "internal_free_before": internal_free},
            "launch_command": ["python3", "-m", "scripts.apple_object_capture.launch",
                               "--probe-bin", str(probe.resolve()),
                               "--images", str(input_dir / "images"),
                               "--run-dir", str(output_dir),
                               "--max-output-mib", "512", "--timeout-minutes", "15"]}


def stage_input(input_dir: Path = INPUT, output_dir: Path = OUTPUT,
                source_dir: Path = bunny.SOURCE, probe: Path = PROBE) -> dict:
    report = preflight(input_dir, output_dir, source_dir, probe)
    input_dir.mkdir()
    images = input_dir / "images"
    images.mkdir()
    report.update(status="staging", started_utc=datetime.now(timezone.utc).isoformat())
    receipt = input_dir / "receipt.json"

    def save() -> None:
        receipt.write_text(json.dumps(report, indent=2) + "\n")

    save()
    try:
        for name, expected in sorted(report["source_photo_sha256"].items()):
            source = source_dir / name
            destination = images / name
            if source.is_symlink() or not source.is_file() or digest(source) != expected:
                raise ValueError(f"processed source changed before copy: {name}")
            shutil.copyfile(source, destination)
            if destination.is_symlink() or digest(destination) != expected:
                raise ValueError(f"staged copy differs: {name}")
            if (folder_bytes(input_dir) > INPUT_CAP or
                    shutil.disk_usage(input_dir).free < FLOOR + OUTPUT_CAP):
                raise ValueError("input cap or external disk floor reached during staging")
        if (len(list(images.iterdir())) != 73 or
                bunny.verify_photos(source_dir)["photo_sha256"] != report["source_photo_sha256"] or
                digest(probe) != PROBE_SHA):
            raise ValueError("source, staged inventory, or probe changed during staging")
        report["staged_photo_sha256"] = {name: digest(images / name)
                                          for name in sorted(report["source_photo_sha256"])}
        report["status"] = "staged_pending_live_review"
    except Exception as error:
        report.update(status="failed", failure=str(error))
    report["finished_utc"] = datetime.now(timezone.utc).isoformat()
    report["staged_input_bytes"] = folder_bytes(input_dir)
    report["external_free_after"] = shutil.disk_usage(input_dir).free
    report["internal_free_after"] = shutil.disk_usage(source_dir).free
    if (report["staged_input_bytes"] > INPUT_CAP or
            report["staged_input_bytes"] + OUTPUT_CAP > TOTAL_CAP or
            report["external_free_after"] < FLOOR + OUTPUT_CAP or
            report["internal_free_after"] < FLOOR):
        report.update(status="failed", resource_postcheck_failure="staged input or disk cap exceeded")
    save()
    return report


def check_staged(input_dir: Path = INPUT, output_dir: Path = OUTPUT,
                 source_dir: Path = bunny.SOURCE, probe: Path = PROBE) -> dict:
    receipt = input_dir / "receipt.json"
    images = input_dir / "images"
    if (input_dir.is_symlink() or images.is_symlink() or receipt.is_symlink() or
            not receipt.is_file() or not images.is_dir() or
            output_dir.exists() or output_dir.is_symlink()):
        raise ValueError("staged input must exist and live output must remain fresh")
    report = json.loads(receipt.read_text())
    photos = bunny.verify_photos(source_dir)
    if (report.get("schema") != SCHEMA or report.get("status") != "staged_pending_live_review" or
            report.get("input_dir") != str(input_dir) or report.get("output_dir") != str(output_dir) or
            report.get("source_photo_sha256") != photos["photo_sha256"] or
            report.get("source_frame_by_photo") != photos["source_frame_by_photo"] or
            report.get("source_prepare_manifest_sha256") != photos["prepare_manifest_sha256"] or
            report.get("probe_sha256") != PROBE_SHA or
            probe.is_symlink() or not probe.is_file() or digest(probe) != PROBE_SHA):
        raise ValueError("staged receipt differs from sealed source or probe")
    if {path.name for path in images.iterdir()} != set(photos["photo_sha256"]):
        raise ValueError("staged image inventory differs from 73 source photographs")
    if any((images / name).is_symlink() or digest(images / name) != expected
           for name, expected in photos["photo_sha256"].items()):
        raise ValueError("staged image bytes differ from sealed source")
    staged_bytes = folder_bytes(input_dir)
    if (staged_bytes > INPUT_CAP or staged_bytes + OUTPUT_CAP > TOTAL_CAP or
            shutil.disk_usage(ROOT).free < FLOOR + OUTPUT_CAP or
            shutil.disk_usage(source_dir).free < FLOOR):
        raise ValueError("staged input plus output cap or 11 GiB disk floor unavailable")
    return {"schema": SCHEMA, "status": "staged_verified_pending_live_review",
            "receipt": str(receipt), "receipt_sha256": digest(receipt),
            "input_dir": str(input_dir), "staged_images": str(images), "images": 73,
            "staged_input_bytes": staged_bytes,
            "max_new_input_plus_output_bytes": staged_bytes + OUTPUT_CAP,
            "output_dir": str(output_dir), "launch_command": report["launch_command"]}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--stage-input", action="store_true")
    mode.add_argument("--check-staged", action="store_true")
    args = parser.parse_args()
    report = (stage_input() if args.stage_input else
              check_staged() if args.check_staged else preflight())
    print(json.dumps({"status": report["status"], "input_dir": report.get("input_dir"),
                      "output_dir": report.get("output_dir"),
                      "failure": report.get("failure")}))
    return 0 if report["status"] != "failed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
