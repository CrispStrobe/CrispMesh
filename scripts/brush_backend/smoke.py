"""Bounded 20-step Brush Metal smoke on the sealed YCB-008 COLMAP input."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time

from .lineage import RUN_008, validate_008
from .ply import validate as validate_splat_ply
from .preflight import MIN_FREE, RELEASE_SHA256, sha256, validate_dataset

ROOT = Path(__file__).resolve().parents[2]
MOUNT = Path("/Volumes/backups")
OUTPUT = MOUNT / "code/crispmesh-data/brush-smoke-001"
BINARY = ROOT / ".local-tools/brush-v030/brush-app-aarch64-apple-darwin/brush_app"
DATASET = RUN_008 / "dense"
MAX_OUTPUT = 512 * 1024**2
MAX_LOG = 16 * 1024**2
MAX_RSS = 4 * 1024**3
MAX_SECONDS = 300
BUFFER = 256 * 1024**2


def command(binary: Path, dataset: Path, output: Path) -> list[str]:
    return [str(binary), str(dataset), "--total-steps", "20", "--max-resolution", "640",
            "--max-splats", "50000", "--seed", "42", "--export-every", "20",
            "--export-path", str(output), "--export-name", "export_{iter}.ply"]


def folder_bytes(path: Path) -> int:
    total = 0
    for base, dirs, files in os.walk(path, followlinks=False):
        for name in dirs + files:
            item = Path(base) / name
            if item.is_symlink():
                raise ValueError(f"symlink in output: {item}")
            if item.is_file():
                total += item.stat().st_size
            elif not item.is_dir():
                raise ValueError(f"special output file: {item}")
    return total


def rss_bytes(pid: int) -> int | None:
    if sys.platform != "darwin":
        return None
    result = subprocess.run(["ps", "-o", "rss=", "-p", str(pid)],
                            capture_output=True, text=True, timeout=2, check=False)
    try:
        return int(result.stdout.strip()) * 1024
    except ValueError:
        return None


def stop_group(proc: subprocess.Popen) -> None:
    if proc.poll() is not None:
        return
    try:
        os.killpg(proc.pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    try:
        proc.wait(timeout=2)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        proc.wait(timeout=2)


def preflight(output: Path = OUTPUT, binary: Path = BINARY,
              dataset: Path = DATASET, mount: Path = MOUNT) -> dict:
    if sys.platform != "darwin" or os.uname().machine != "arm64":
        raise ValueError("this pinned Metal smoke is Apple Silicon only")
    if not mount.is_mount() or mount.stat().st_dev == ROOT.stat().st_dev:
        raise ValueError("external output volume not mounted as separate device")
    if output != OUTPUT or output.exists() or output.is_symlink() or output.parent.is_symlink():
        raise ValueError("output must be exact fresh approved external path")
    if not output.resolve().is_relative_to(mount.resolve()):
        raise ValueError("output escapes external mount")
    if not binary.is_file() or binary.is_symlink() or sha256(binary) != "8380ed40cce870025393e1ea0257e0752351c67a409d827b76dc75ec3a999a71":
        raise ValueError("release binary hash mismatch")
    data = validate_dataset(dataset, output, expected_result=RUN_008 / "result.json")
    lineage = validate_008(dataset, data["image_sha256"], data["model_file_sha256"])
    if lineage["source_undistort_stage"] != "complete":
        raise ValueError("source undistort stage not complete")
    if shutil.disk_usage(mount).free < MIN_FREE + MAX_OUTPUT + BUFFER:
        raise ValueError("external 10 GiB floor plus output budget not met")
    if shutil.disk_usage(ROOT).free < MIN_FREE + BUFFER:
        raise ValueError("internal 10 GiB floor plus temp buffer not met")
    return {"schema": "brush_smoke_preflight_v1", "binary_sha256": sha256(binary),
            "release_archive_sha256": RELEASE_SHA256, "dataset": data,
            "lineage": lineage, "training_split": "all 60 registered images; zero in-dataset eval split",
            "external_heldout_inventory": "not claimed", "quality_claim": False,
            "command": command(binary, dataset, output),
            "limits": {"seconds": MAX_SECONDS, "output_bytes": MAX_OUTPUT,
                       "log_bytes": MAX_LOG, "sampled_process_rss_bytes": MAX_RSS,
                       "free_bytes_floor": MIN_FREE}}


def run() -> dict:
    report = preflight()
    temp_dir = ROOT / ".local-tools/tmp/brush-smoke-001"
    if temp_dir.exists() or temp_dir.is_symlink():
        raise ValueError("fresh project-local TMPDIR required")
    OUTPUT.mkdir()
    temp_dir.mkdir(parents=True)
    log = OUTPUT / "train.log"
    start = time.monotonic()
    proc = None
    failure = None
    peak_rss = None
    returncode = None
    before = {"images": report["dataset"]["image_sha256"],
              "model": report["dataset"]["model_file_sha256"]}
    try:
        env = os.environ.copy()
        env["TMPDIR"] = str(temp_dir)
        with log.open("xb") as stream:
            proc = subprocess.Popen(report["command"], cwd=OUTPUT, env=env,
                                    stdout=stream, stderr=subprocess.STDOUT,
                                    start_new_session=True)
            while proc.poll() is None:
                elapsed = time.monotonic() - start
                current_rss = rss_bytes(proc.pid)
                if current_rss is not None:
                    peak_rss = max(peak_rss or 0, current_rss)
                if elapsed > MAX_SECONDS:
                    failure = "timeout"
                elif current_rss is not None and current_rss > MAX_RSS:
                    failure = "sampled process RSS cap"
                elif folder_bytes(OUTPUT) + folder_bytes(temp_dir) > MAX_OUTPUT:
                    failure = "output+temporary byte cap"
                elif log.stat().st_size > MAX_LOG:
                    failure = "log byte cap"
                elif shutil.disk_usage(MOUNT).free < MIN_FREE or shutil.disk_usage(ROOT).free < MIN_FREE:
                    failure = "10 GiB disk floor"
                if failure:
                    stop_group(proc)
                    break
                time.sleep(0.25)
            returncode = proc.wait(timeout=3)
    except Exception as exc:
        failure = f"launcher exception: {type(exc).__name__}: {exc}"
    finally:
        if proc is not None:
            stop_group(proc)
    try:
        after = validate_dataset(DATASET, OUTPUT / "unused-fresh-output-for-rehash")
        if after["image_sha256"] != before["images"] or after["model_file_sha256"] != before["model"]:
            failure = failure or "input hashes changed"
        validate_008(DATASET, after["image_sha256"], after["model_file_sha256"])
    except Exception as exc:
        failure = failure or f"input postcheck: {exc}"
    exports = sorted(p for p in OUTPUT.glob("export_*.ply") if p.is_file() and not p.is_symlink())
    if returncode != 0:
        failure = failure or f"Brush exit {returncode}"
    if not exports or any(p.stat().st_size == 0 for p in exports):
        failure = failure or "no nonempty splat PLY export"
    export_validation = {}
    for path in exports:
        try:
            export_validation[path.name] = validate_splat_ply(path)
        except ValueError as exc:
            failure = failure or f"invalid splat PLY {path.name}: {exc}"
    if folder_bytes(OUTPUT) + folder_bytes(temp_dir) > MAX_OUTPUT:
        failure = failure or "postflight output+temporary byte cap"
    if log.exists() and log.stat().st_size > MAX_LOG:
        failure = failure or "postflight log byte cap"
    if sha256(BINARY) != report["binary_sha256"]:
        failure = failure or "release binary hash changed"
    if shutil.disk_usage(MOUNT).free < MIN_FREE or shutil.disk_usage(ROOT).free < MIN_FREE:
        failure = failure or "postflight 10 GiB disk floor"
    report.update({"status": "failed" if failure else "complete", "failure": failure,
                   "returncode": returncode, "elapsed_seconds": round(time.monotonic() - start, 3),
                   "peak_sampled_process_rss_bytes": peak_rss,
                   "output_bytes": folder_bytes(OUTPUT), "temporary_bytes": folder_bytes(temp_dir),
                   "exports": {p.name: {"bytes": p.stat().st_size, "sha256": sha256(p)} for p in exports},
                   "export_validation": export_validation,
                   "log_sha256": sha256(log) if log.exists() else None,
                   "gpu_unified_memory_measured": False,
                   "artifact_validity": "finite bounded Gaussian-splat PLY structure only; no geometry or quality validation"})
    with (OUTPUT / "report.json").open("x") as stream:
        json.dump(report, stream, indent=2, sort_keys=True)
        stream.write("\n")
    return report


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preflight", action="store_true", help="read-only: print command, lineage, and budgets")
    args = parser.parse_args()
    report = preflight() if args.preflight else run()
    print(json.dumps(report, indent=2, sort_keys=True))
    if report.get("status") == "failed":
        raise SystemExit(1)


if __name__ == "__main__":
    main()
