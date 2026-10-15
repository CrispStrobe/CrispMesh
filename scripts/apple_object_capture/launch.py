"""Bounded macOS-only launcher for the original RealityKit Photogrammetry probe.

Support checks are cheap. Reconstruction is opt-in and intentionally separate
from the cross-platform CrispMesh pipeline.
"""
from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
import shutil
import sys
import time

from scripts.classical_backend.run import RESERVE, StageError, digest, folder_bytes, stage

MAX_OUTPUT_MIB = 1024
MAX_TIMEOUT_MINUTES = 30
MAX_IMAGES = 500
IMAGE_EXTENSIONS = {".jpg", ".jpeg", ".png"}
PREFLIGHT_BUFFER = 256 << 20


def _photos(directory: Path) -> list[dict]:
    if directory.is_symlink() or not directory.is_dir():
        raise ValueError("images must be a real directory")
    entries = sorted(directory.iterdir())
    if any(path.suffix.lower() not in IMAGE_EXTENSIONS or not path.is_file() for path in entries):
        raise ValueError("input folder must contain only supported top-level image files")
    paths = entries
    if not 3 <= len(paths) <= MAX_IMAGES:
        raise ValueError("expected 3–500 RGB photographs")
    if any(path.is_symlink() or not path.is_file() or path.stat().st_size == 0 for path in paths):
        raise ValueError("image is missing, empty, or linked")
    return [{"name": path.name, "sha256": digest(path), "bytes": path.stat().st_size}
            for path in paths]


def launch(binary: Path, run_dir: Path, *, images: Path | None = None,
           max_output_mib: int | None = None,
           timeout_minutes: float = 15.0) -> dict:
    if sys.platform != "darwin":
        raise ValueError("Apple PhotogrammetrySession launcher requires macOS")
    if max_output_mib is None:
        max_output_mib = 100 if images is None else MAX_OUTPUT_MIB
    if (not 1 <= max_output_mib <= MAX_OUTPUT_MIB or
            not math.isfinite(timeout_minutes) or
            not 0 < timeout_minutes <= MAX_TIMEOUT_MINUTES):
        raise ValueError("invalid output or timeout cap")
    if binary.is_symlink() or not binary.is_file():
        raise ValueError("probe binary missing or linked")
    if run_dir.exists() or run_dir.is_symlink() or not run_dir.parent.is_dir() or run_dir.parent.is_symlink():
        raise ValueError("run directory must be fresh under a real parent")
    binary = binary.resolve()
    run_dir = run_dir.resolve()
    if images is not None:
        if images.is_symlink():
            raise ValueError("images directory is linked")
        images = images.resolve()
        if images == run_dir or images in run_dir.parents or run_dir in images.parents:
            raise ValueError("run directory must be separate from source images")
        photos = _photos(images)
    else:
        photos = []
    source_volumes = (binary.parent,) if images is None else (binary.parent, images)
    cap = max_output_mib << 20
    if shutil.disk_usage(run_dir.parent).free < RESERVE + cap + PREFLIGHT_BUFFER:
        raise ValueError("insufficient free space for output cap and 10 GiB reserve")
    if any(shutil.disk_usage(path).free < RESERVE + PREFLIGHT_BUFFER
           for path in source_volumes):
        raise ValueError("insufficient free space for source-volume 10 GiB reserve")
    command = [str(binary), "--check-support"] if images is None else [
        str(binary), "--images", str(images), "--output", str(run_dir / "model.usdz")]
    run_dir.mkdir()
    report = {"schema": "apple_photogrammetry_probe_v1", "status": "running",
              "mode": "support" if images is None else "rgb_to_usdz",
              "binary_sha256": digest(binary), "inputs": photos, "command": command,
              "limits": {"max_output_bytes": cap, "max_log_bytes": 16 << 20,
                         "max_child_rss_bytes": 8 << 30,
                         "timeout_minutes": timeout_minutes, "min_free_bytes": RESERVE,
                         "source_reserve_paths": [str(path) for path in source_volumes]},
              "stages": [], "apple_only": True, "cross_platform_backend": False,
              "quality_accepted": False}
    report_path = run_dir / "result.json"
    report_path.write_text(json.dumps(report, indent=2) + "\n")
    deadline = time.monotonic() + timeout_minutes * 60
    try:
        if digest(binary) != report["binary_sha256"]:
            raise ValueError("probe binary changed before launch")
        result = stage(run_dir, "photogrammetry", command, deadline, cap, 16 << 20,
                       8 << 30, source_volumes)
        report["stages"].append(result)
        if images is None:
            support_text = (run_dir / "photogrammetry.log").read_text(errors="replace")
            support = next((json.loads(line) for line in support_text.splitlines()
                            if line.startswith('{"schema":"apple_photogrammetry_support_v1"')), None)
            if support is None or support.get("supported") is not True:
                raise ValueError("support probe gave no positive hardware result")
            report["support"] = support
        else:
            output = run_dir / "model.usdz"
            if output.is_symlink() or not output.is_file() or output.stat().st_size == 0:
                raise ValueError("PhotogrammetrySession did not produce a nonempty USDZ")
        report["status"] = "complete"
    except StageError as error:
        report["stages"].append(error.result)
        report.update(status="failed", failure=str(error))
    except Exception as error:
        report.update(status="failed", failure=str(error))
    try:
        report["binary_unchanged"] = binary.is_file() and not binary.is_symlink() and \
            digest(binary) == report["binary_sha256"]
    except (OSError, ValueError):
        report["binary_unchanged"] = False
    try:
        report["source_images_unchanged"] = (_photos(images) == photos
                                             if images is not None else None)
    except (OSError, ValueError):
        report["source_images_unchanged"] = False
    if not report["binary_unchanged"] or report["source_images_unchanged"] is False:
        report.update(status="failed", provenance_failure="binary or source images changed")
    try:
        report["output_bytes"] = folder_bytes(run_dir)
    except ValueError as error:
        report["output_bytes"] = None
        report.update(status="failed", output_inventory_failure=str(error))
    report["free_bytes_after"] = shutil.disk_usage(run_dir).free
    report["source_free_bytes_after"] = {str(path): shutil.disk_usage(path).free
                                         for path in source_volumes}
    if ((report["output_bytes"] is not None and report["output_bytes"] > cap) or
            report["free_bytes_after"] < RESERVE or
            any(free < RESERVE for free in report["source_free_bytes_after"].values())):
        report.update(status="failed", resource_postcheck_failure="output cap or disk reserve exceeded")
    if images is not None:
        output = run_dir / "model.usdz"
        if output.is_symlink():
            report["artifact"] = {"path": str(output), "validity": "rejected_symlink"}
            report.update(status="failed", artifact_failure="USDZ output is a symlink")
        elif output.is_file():
            report["artifact"] = {"path": str(output), "bytes": output.stat().st_size,
                                  "sha256": digest(output),
                                  "validity": ("nonempty_file_only; USDZ contents and mesh not inspected"
                                               if report["status"] == "complete" else
                                               "partial_or_unverified; USDZ contents and mesh not inspected")}
    report_path.write_text(json.dumps(report, indent=2) + "\n")
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--probe-bin", required=True, type=Path)
    parser.add_argument("--run-dir", required=True, type=Path)
    parser.add_argument("--check-support", action="store_true")
    parser.add_argument("--images", type=Path)
    parser.add_argument("--max-output-mib", type=int)
    parser.add_argument("--timeout-minutes", type=float, default=15.0)
    args = parser.parse_args()
    if args.check_support == bool(args.images):
        parser.error("choose exactly one of --check-support or --images")
    report = launch(args.probe_bin, args.run_dir, images=args.images,
                    max_output_mib=args.max_output_mib,
                    timeout_minutes=args.timeout_minutes)
    print(json.dumps({"status": report["status"], "result": str(args.run_dir / "result.json"),
                      "support": report.get("support"), "failure": report.get("failure")}))
    return 0 if report["status"] == "complete" else 1


if __name__ == "__main__":
    raise SystemExit(main())
