"""One-shot, evaluation-only OpenMVG Sceaux photo SfM control.

Default invocation is read-only. The run path requires five locally built CLIs,
including the separately reviewed GeometricFilter target. No reference pose,
mesh, depth, or held-out image is read by the worker.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
try:
    import resource
except ImportError:  # Windows test discovery; native execution is macOS-only.
    resource = None
import shutil
import signal
import subprocess
import sys
import time

from scripts.classical_backend import openmvg_m1_supervisor as base
from scripts.upstream_control import fetch

BUILD = Path("/Volumes/backups/code/crispmesh-data/openmvg-v21-ligt-off-oracle-001")
OUTPUT = Path("/Volumes/backups/code/crispmesh-data/openmvg-sceaux-photo-sfm-001")
SAMPLE = fetch.OUTPUT
SAMPLE_MANIFEST_SHA = "1ee34cff7e6d75fca9f56de4aa5644b45a382030af54a474824ada6af77316a3"
FOUR_RECEIPT_SHA = "9aec9a7ac01c76f683416cd8eb47c57bbcbfa006a642348568a04298570078c1"
FIFTH_MANIFEST_SHA = "4b3dc5ac03ea2232067bebabc0b4d41dc5bfb91076bf0e31211162f32842774b"
FIFTH_RECEIPT_SHA = "49db5dca401c827cac53a5b1337256effd198ed36de23de283b2b32548e53dc2"
FIFTH_BINARY_SHA = "49a5ca029356f3f8bb3eb68eaae58d98f77204b2b4cdf85f52aa0448f0ab1b77"
NAMES = tuple(f"{i:05}.jpg" for i in range(11))
TARGETS = (*base.TARGETS[:3], "openMVG_main_GeometricFilter", base.TARGETS[3])
BIN_DIR = BUILD / "build/Darwin-arm64-Release"
OUTPUT_CAP = 1 << 30
RSS_CAP_KIB = 4 * (1 << 20)
LOG_CAP = 16 << 20
FLOOR = 10 << 30
MARGIN = 1 << 30
TOTAL_SECONDS = 1800
STAGE_SECONDS = (120, 420, 420, 420, 420)


def sha(path: Path) -> str:
    return base.sha256(path)


def verify_photos(sample: Path = SAMPLE) -> dict:
    manifest_file = sample / "manifest.json"
    if manifest_file.is_symlink() or sha(manifest_file) != SAMPLE_MANIFEST_SHA:
        raise RuntimeError("upstream Sceaux manifest seal mismatch")
    manifest = json.loads(manifest_file.read_text())
    records = {Path(row["path"]).name: row for row in manifest["files"]
               if row["path"].startswith("images/")}
    image_dir = sample / "images"
    if (set(records) != set(NAMES) or image_dir.is_symlink() or
            {item.name for item in image_dir.iterdir()} != set(NAMES)):
        raise RuntimeError("Sceaux image inventory differs from the sealed 11")
    for name in NAMES:
        item = image_dir / name
        row = records[name]
        if item.is_symlink() or item.stat().st_size != row["bytes"] or sha(item) != row["sha256"]:
            raise RuntimeError(f"Sceaux photo seal mismatch: {name}")
    return {"manifest_sha256": SAMPLE_MANIFEST_SHA,
            "photos": {name: records[name]["sha256"] for name in NAMES},
            "total_photo_bytes": sum(records[name]["bytes"] for name in NAMES)}


def verify_binaries(build: Path = BUILD) -> dict:
    if build.is_symlink() or not build.is_dir():
        raise RuntimeError("pinned OpenMVG build directory unavailable")
    # Both exact one-shot build receipts are independently sealed.
    first_receipt = build / "license-closure-receipt.json"
    fifth_receipt = build / "geometric-filter-license-receipt.json"
    fifth_manifest = build / "geometric-filter-build-manifest.json"
    for receipt in (first_receipt, fifth_receipt, fifth_manifest):
        if receipt.is_symlink() or not receipt.is_file():
            raise RuntimeError(f"reviewed build receipt missing: {receipt}")
    if sha(first_receipt) != FOUR_RECEIPT_SHA:
        raise RuntimeError("four-target build receipt seal mismatch")
    if sha(fifth_receipt) != FIFTH_RECEIPT_SHA or sha(fifth_manifest) != FIFTH_MANIFEST_SHA:
        raise RuntimeError("fifth-target build manifest/receipt seal mismatch")
    first = json.loads(first_receipt.read_text())
    fifth = json.loads(fifth_receipt.read_text())
    manifest = json.loads(fifth_manifest.read_text())
    if (manifest.get("schema") != "openmvg_geometric_filter_build_v1" or
            manifest.get("status") != "built_fifth_oracle_only" or
            manifest.get("receipt_sha256") != FIFTH_RECEIPT_SHA or
            len(manifest.get("stages", [])) != 1 or
            manifest["stages"][0].get("status") != "completed" or
            manifest["stages"][0].get("returncode") != 0 or
            manifest.get("input", {}).get("input_sha256", {}).get(str(first_receipt)) != FOUR_RECEIPT_SHA):
        raise RuntimeError("fifth-target build manifest contract differs")
    binaries = {}
    for name in TARGETS:
        item = build / "build/Darwin-arm64-Release" / name
        if item.is_symlink() or not item.is_file() or not os.access(item, os.X_OK):
            raise RuntimeError(f"required reviewed OpenMVG target missing: {name}")
        binaries[name] = sha(item)
        if name in base.TARGETS and first.get("binaries", {}).get(name, {}).get("sha256") != binaries[name]:
            raise RuntimeError(f"four-target binary differs from sealed receipt: {name}")
    if (fifth.get("schema") != "openmvg_geometric_filter_license_receipt_v1" or
            fifth.get("target") != "openMVG_main_GeometricFilter" or
            fifth.get("decision") != "evaluation oracle only; no commercial/App Store clearance" or
            fifth.get("four_target_receipt_sha256") != FOUR_RECEIPT_SHA or
            fifth.get("binary", {}).get("path") != str(build / "build/Darwin-arm64-Release/openMVG_main_GeometricFilter") or
            fifth.get("binary", {}).get("sha256") != FIFTH_BINARY_SHA or
            binaries["openMVG_main_GeometricFilter"] != FIFTH_BINARY_SHA):
        raise RuntimeError("fifth-target binary/receipt contract differs")
    return {"binary_sha256": binaries,
            "four_target_receipt_sha256": FOUR_RECEIPT_SHA,
            "fifth_manifest_sha256": FIFTH_MANIFEST_SHA,
            "fifth_receipt_sha256": FIFTH_RECEIPT_SHA}


def capacity(output: Path, prospective: bool = False) -> dict:
    used = base.tree_bytes(output)
    external = base.free_bytes(output)
    internal = base.free_bytes(base.DEFAULT_INTERNAL)
    needed = FLOOR + MARGIN + (OUTPUT_CAP - used if prospective else 0)
    if used > OUTPUT_CAP or external < needed or internal < FLOOR + MARGIN:
        raise RuntimeError("OpenMVG photo control disk/output cap breached")
    return {"output_bytes": used, "external_free": external,
            "external_required": needed, "internal_free": internal}


def preflight(output: Path = OUTPUT, sample: Path = SAMPLE, build: Path = BUILD) -> dict:
    if output.exists() or output.is_symlink():
        raise RuntimeError("one-shot output path must be fresh")
    if not output.parent.is_dir() or output.parent.is_symlink():
        raise RuntimeError("external output parent must exist and be real")
    if output.parent.resolve() != BUILD.parent.resolve():
        raise RuntimeError("output must be a fresh sibling on the external device")
    photos = verify_photos(sample)
    binaries = verify_binaries(build)
    return {"schema": "openmvg_sceaux_photo_control_v1", "status": "read_only_preflight",
            "source": str(sample / "images"), "output": str(output),
            "photos": photos, "binaries": binaries, "capacity": capacity(output, True),
            "limits": {"output_bytes": OUTPUT_CAP, "rss_kib": RSS_CAP_KIB,
                       "log_bytes_per_stage": LOG_CAP, "wall_seconds_total": TOTAL_SECONDS,
                       "wall_seconds_per_stage": STAGE_SECONDS, "threads": 2},
            "license_scope": "evaluation only; no shipping or GPL/AGPL clearance"}


def commands(output: Path, build: Path = BUILD) -> list[tuple[str, list[str], str]]:
    binaries = [str(build / "build/Darwin-arm64-Release" / name) for name in TARGETS]
    images, matches, sparse = (output / name for name in ("images", "matches", "sparse"))
    data = str(matches / "sfm_data.json")
    putative = str(matches / "matches.putative.bin")
    geometric = str(matches / "matches.e.bin")
    return [
        ("listing", [binaries[0], "-i", str(images), "-o", str(matches), "-f", "3398.4",
                     "-c", "2", "-g", "1"], data),
        ("features", [binaries[1], "-i", data, "-o", str(matches), "-m", "SIFT_ANATOMY", "-p", "NORMAL", "-n", "2"], str(matches / "image_describer.json")),
        ("putative", [binaries[2], "-i", data, "-o", putative, "-r", "0.8"], putative),
        ("geometric", [binaries[3], "-i", data, "-m", putative, "-o", geometric, "-g", "e"], geometric),
        ("sfm", [binaries[4], "-i", data, "-m", str(matches), "-M", "matches.e.bin",
                 "-o", str(sparse), "-s", "INCREMENTAL", "-f", "ADJUST_ALL"], str(sparse / "sfm_data.bin")),
    ]


def output_inventory(output: Path) -> dict:
    inventory = {}
    for directory, dirs, files in os.walk(output, followlinks=False):
        for name in dirs + files:
            item = Path(directory) / name
            if item.is_symlink():
                raise RuntimeError("symlink found in OpenMVG output")
            if item.is_file() and item != output / "receipt.json":
                inventory[item.relative_to(output).as_posix()] = {"bytes": item.stat().st_size,
                                                                  "sha256": sha(item)}
    return inventory


def report_counts(report: Path) -> dict:
    """Read the five scalar counts emitted by pinned sfm_report.cpp."""
    if report.is_symlink() or not report.is_file() or report.stat().st_size > LOG_CAP:
        raise RuntimeError("missing or oversized SfM HTML report")
    html = report.read_text(errors="replace")
    counts = {}
    for key in ("views", "poses", "intrinsics", "tracks", "residuals"):
        values = re.findall(r"#" + key + r":\s*(\d+)\s*<br\s*/?>", html)
        if len(values) != 1:
            raise RuntimeError(f"SfM report has no unique #{key} count")
        counts[key] = int(values[0])
    if counts["views"] != len(NAMES) or counts["poses"] > counts["views"]:
        raise RuntimeError("SfM report view/pose counts are inconsistent")
    return counts


def save(output: Path, receipt: dict) -> None:
    (output / "receipt.json").write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n")


def run_stage(command: list[str], log: Path, output: Path, seconds: int,
              total_start: float) -> dict:
    if sys.platform != "darwin" or resource is None:
        raise RuntimeError("OpenMVG native execution requires macOS resource limits")
    started = time.monotonic()
    env = os.environ.copy()
    env.update({"OMP_NUM_THREADS": "2", "OPENBLAS_NUM_THREADS": "2",
                "VECLIB_MAXIMUM_THREADS": "2", "TMPDIR": str(output / "tmp"),
                "TMP": str(output / "tmp"), "TEMP": str(output / "tmp")})
    peak = 0
    def set_cpu_limit() -> None:
        # Two allowed worker threads may consume two CPU seconds per wall second.
        resource.setrlimit(resource.RLIMIT_CPU, (2 * seconds, 2 * seconds + 1))

    with log.open("xb") as stream:
        proc = subprocess.Popen(command, stdout=stream, stderr=subprocess.STDOUT,
                                env=env, start_new_session=True, preexec_fn=set_cpu_limit)
        try:
            while proc.poll() is None:
                elapsed = time.monotonic() - started
                if elapsed > seconds or time.monotonic() - total_start > TOTAL_SECONDS:
                    raise RuntimeError("stage/total time cap exceeded")
                if log.stat().st_size > LOG_CAP:
                    raise RuntimeError("stage log cap exceeded")
                capacity(output)
                table = subprocess.check_output(["/bin/ps", "-axo", "pid=,ppid=,rss="], text=True)
                peak = max(peak, base.process_rss_kib(proc.pid, table))
                if peak > RSS_CAP_KIB:
                    raise RuntimeError("process-tree RSS cap exceeded")
                time.sleep(0.5)
            if proc.returncode or log.stat().st_size > LOG_CAP:
                raise RuntimeError(f"stage returned {proc.returncode} or log exceeded cap")
            capacity(output)
        except BaseException:
            if proc.poll() is None:
                os.killpg(proc.pid, signal.SIGTERM)
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(proc.pid, signal.SIGKILL)
                    proc.wait(timeout=5)
            raise
    return {"command": command, "wall_seconds": round(time.monotonic() - started, 3),
            "peak_rss_kib": peak, "log_sha256": sha(log), "log_bytes": log.stat().st_size}


def run(output: Path = OUTPUT) -> dict:
    if sys.platform != "darwin" or resource is None:
        raise RuntimeError("OpenMVG native execution requires macOS resource limits")
    checked = preflight(output)
    output.mkdir()
    for name in ("images", "matches", "sparse", "logs", "tmp"):
        (output / name).mkdir()
    receipt = {**checked, "status": "running", "started_utc": datetime.now(timezone.utc).isoformat(),
               "stages": []}
    save(output, receipt)
    try:
        for name in NAMES:
            src, dst = SAMPLE / "images" / name, output / "images" / name
            shutil.copyfile(src, dst)
            if sha(dst) != checked["photos"]["photos"][name]:
                raise RuntimeError(f"staged photo differs: {name}")
            capacity(output)
        if verify_photos() != checked["photos"]:
            raise RuntimeError("sealed photos changed during staging")
        receipt["staged_photo_sha256"] = {name: sha(output / "images" / name) for name in NAMES}
        save(output, receipt)
        total_start = time.monotonic()
        for index, (name, command, expected) in enumerate(commands(output)):
            record = {"name": name, "status": "running", "command": command}
            receipt["stages"].append(record)
            save(output, receipt)
            try:
                record.update(run_stage(command, output / "logs" / f"{index + 1:02}-{name}.log",
                                        output, STAGE_SECONDS[index], total_start))
                artifact = Path(expected)
                if artifact.is_symlink() or not artifact.is_file() or artifact.stat().st_size == 0:
                    raise RuntimeError(f"required stage artifact missing: {artifact}")
                record["artifact_sha256"] = sha(artifact)
                record["status"] = "completed"
            except BaseException as exc:
                record["status"] = "stopped"
                record["reason"] = str(exc)
                raise
            finally:
                record["output_bytes"] = base.tree_bytes(output)
                save(output, receipt)
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
        receipt["output_inventory"] = output_inventory(output)
        save(output, receipt)
    return receipt


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run-sceaux", action="store_true", help="one-shot live photo run; requires separate review")
    args = parser.parse_args()
    result = run() if args.run_sceaux else preflight()
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
