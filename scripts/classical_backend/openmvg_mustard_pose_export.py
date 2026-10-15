"""One-shot diagnostic JSON export of the sealed failed mustard SfM model.

Default invocation is read-only. This never repairs or reruns SfM.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
import math
import os
from pathlib import Path
try:
    import resource
except ImportError:  # Windows test discovery; native execution is macOS-only.
    resource = None
import shutil
import signal
import subprocess
import sys
import time

from scripts.classical_backend import openmvg_converter_build as build

FAILED = Path("/Volumes/backups/code/crispmesh-data/openmvg-mustard-photo-sfm-001")
OUTPUT = Path("/Volumes/backups/code/crispmesh-data/openmvg-mustard-photo-pose-export-001")
INPUT = FAILED / "sparse/sfm_data.bin"
FAIL_RECEIPT_SHA = "e45c400dcf71a3ba86fa22f7af2138418d6dc511fddcd6d8470ef33599804744"
INPUT_SHA = "dc11f9b3a74809ebc080260b360ff1dd0e6f9f9526ea151aac002ace75b2df8b"
BUILD_MANIFEST_SHA = "e7f0b3cce52612466756aad53231eb6e1b2fc83b884bec829e548f9f6b4a22fa"
BUILD_RECEIPT_SHA = "51f8e5855bc7158f27cfdcbd6a84c4bee594e9ee24d42155d7d7837f43290bf4"
BUILD_LOG_SHA = "c036acd2e1162bd7afbcff48d187900d5dc3465c1752e2a2360ea195760d6b35"
BINARY_SHA = "95bebb65afd1374aadae21aecc5f4cf66f4257432477d54ceae22c4cc35636b0"
OUTPUT_CAP = 64 << 20
LOG_CAP = 4 << 20
RSS_CAP_KIB = 1 << 20
TIMEOUT = 120
FLOOR = 11 << 30


def sha(path: Path) -> str:
    return build.fifth.four.fork.base.sha256(path)


def verify_failed_model(failed: Path = FAILED) -> dict:
    receipt_file, model = failed / "receipt.json", failed / "sparse/sfm_data.bin"
    if (failed.is_symlink() or not failed.is_dir() or receipt_file.is_symlink() or
            model.is_symlink() or sha(receipt_file) != FAIL_RECEIPT_SHA or
            sha(model) != INPUT_SHA):
        raise RuntimeError("sealed failed mustard SfM receipt/model differs")
    receipt = json.loads(receipt_file.read_text())
    report = receipt.get("sfm_report", {})
    if (receipt.get("status") != "failed_registration_or_sparse_gate" or
            (report.get("views"), report.get("poses"), report.get("tracks"), report.get("residuals")) !=
            (48, 46, 222, 1497) or
            [stage.get("status") for stage in receipt.get("stages", [])] != ["completed"] * 5 or
            receipt.get("output_inventory", {}).get("sparse/sfm_data.bin", {}).get("sha256") != INPUT_SHA):
        raise RuntimeError("failed mustard gate is not the sealed 46/48 model")
    return {"receipt_sha256": FAIL_RECEIPT_SHA, "sfm_data_bin_sha256": INPUT_SHA,
            "sfm_report": report, "status": receipt["status"]}


def verify_converter() -> dict:
    files = {build.MANIFEST: BUILD_MANIFEST_SHA, build.RECEIPT: BUILD_RECEIPT_SHA,
             build.LOG: BUILD_LOG_SHA, build.BINARY: BINARY_SHA}
    for item, expected in files.items():
        if item.is_symlink() or not item.is_file() or sha(item) != expected:
            raise RuntimeError(f"sealed sixth-target build artifact differs: {item}")
    manifest = json.loads(build.MANIFEST.read_text())
    receipt = json.loads(build.RECEIPT.read_text())
    if (manifest.get("schema") != "openmvg_converter_build_v1" or
            manifest.get("status") != "built_sixth_oracle_only" or
            manifest.get("receipt_sha256") != BUILD_RECEIPT_SHA or
            len(manifest.get("stages", [])) != 1 or
            manifest["stages"][0].get("status") != "completed" or
            manifest["stages"][0].get("returncode") != 0 or
            manifest["stages"][0].get("log_sha256") != BUILD_LOG_SHA or
            receipt.get("schema") != "openmvg_converter_license_receipt_v1" or
            receipt.get("target") != build.TARGET or
            receipt.get("binary", {}).get("sha256") != BINARY_SHA or
            receipt.get("binary", {}).get("path") != str(build.BINARY) or
            receipt.get("new_static_or_dynamic_libraries") != [] or
            build.closure_receipt() != receipt):
        raise RuntimeError("sixth-target build/license closure contract differs")
    return {"build_manifest_sha256": BUILD_MANIFEST_SHA,
            "build_receipt_sha256": BUILD_RECEIPT_SHA,
            "build_log_sha256": BUILD_LOG_SHA, "binary_sha256": BINARY_SHA}


def capacity(output: Path, prospective: bool = False) -> dict:
    used = build.fifth.four.fork.base.tree_bytes(output)
    external = build.fifth.four.fork.base.free_bytes(output)
    internal = build.fifth.four.fork.base.free_bytes(build.fifth.four.fork.base.DEFAULT_INTERNAL)
    required = FLOOR + (OUTPUT_CAP - used if prospective else 0)
    if used > OUTPUT_CAP or external < required or internal < FLOOR:
        raise RuntimeError("pose export disk/output cap breached")
    return {"output_bytes": used, "external_free": external,
            "external_required": required, "internal_free": internal}


def command(output: Path = OUTPUT) -> list[str]:
    return [str(build.BINARY), "-i", str(INPUT),
            "-o", str(output / "sfm_camera_poses.json"), "-V", "-I", "-E"]


def preflight(output: Path = OUTPUT) -> dict:
    if output.exists() or output.is_symlink():
        raise RuntimeError("one-shot pose export output must be fresh")
    if (not output.parent.is_dir() or output.parent.is_symlink() or
            output.parent.resolve() != build.ROOT.parent.resolve()):
        raise RuntimeError("pose export must be a fresh external sibling")
    failed = verify_failed_model()
    converter = verify_converter()
    space = capacity(output, prospective=True)
    return {"schema": "openmvg_mustard_pose_export_v1", "status": "read_only_preflight",
            "failed_model": failed, "converter": converter, "output": str(output),
            "command": command(output), "capacity": space,
            "limits": {"output_bytes": OUTPUT_CAP, "log_bytes": LOG_CAP,
                       "rss_kib": RSS_CAP_KIB, "wall_seconds": TIMEOUT,
                       "cpu_seconds": TIMEOUT, "disk_floor_bytes": FLOOR},
            "role": "diagnostic export of failed 46/48 pose model; no reference data"}


def _entries(value: object, label: str) -> list[dict]:
    if not isinstance(value, list) or any(not isinstance(entry, dict) or
                                          "key" not in entry or "value" not in entry
                                          for entry in value):
        raise RuntimeError(f"invalid exported {label} map")
    return value


def _view_data(value: object) -> dict:
    if not isinstance(value, dict):
        raise RuntimeError("invalid exported view")
    wrapped = value.get("ptr_wrapper")
    if not isinstance(wrapped, dict) or not isinstance(wrapped.get("data"), dict):
        raise RuntimeError("invalid exported view wrapper")
    return wrapped["data"]


def inspect_json(path: Path, expected_names: list[str]) -> dict:
    if path.is_symlink() or not path.is_file() or path.stat().st_size > OUTPUT_CAP:
        raise RuntimeError("missing or oversized pose JSON")
    data = json.loads(path.read_text())
    views = _entries(data.get("views"), "views")
    poses = _entries(data.get("extrinsics"), "extrinsics")
    intrinsics = _entries(data.get("intrinsics"), "intrinsics")
    if (len(views) != 48 or len(poses) != 46 or not intrinsics or
            data.get("structure") not in ([], {}) or
            data.get("control_points") not in ([], {})):
        raise RuntimeError("exported field/count gate differs from failed model")
    names, pose_ids, view_ids = [], [], []
    for entry in views:
        value = _view_data(entry["value"])
        names.append(value.get("filename"))
        pose_ids.append(value.get("id_pose"))
        view_ids.append(value.get("id_view"))
    if (len(set(names)) != 48 or set(names) != set(expected_names) or
            len(set(view_ids)) != 48 or len(set(entry["key"] for entry in views)) != 48):
        raise RuntimeError("exported views differ from sealed TRAIN names/IDs")
    keyed_poses = {entry["key"]: entry["value"] for entry in poses}
    if len(keyed_poses) != 46 or set(keyed_poses) - set(pose_ids):
        raise RuntimeError("exported poses have duplicate or unattached IDs")
    for pose in keyed_poses.values():
        if not isinstance(pose, dict):
            raise RuntimeError("invalid pose value")
        rotation, center = pose.get("rotation"), pose.get("center")
        if (not isinstance(rotation, list) or len(rotation) != 3 or
                any(not isinstance(row, list) or len(row) != 3 for row in rotation) or
                not isinstance(center, list) or len(center) != 3):
            raise RuntimeError("invalid pose matrix/center dimensions")
        values = [float(value) for row in rotation for value in row] + [float(value) for value in center]
        if not all(math.isfinite(value) for value in values):
            raise RuntimeError("nonfinite exported pose")
        rows = [[float(v) for v in row] for row in rotation]
        for i in range(3):
            for j in range(3):
                dot = sum(rows[i][k] * rows[j][k] for k in range(3))
                if abs(dot - (1.0 if i == j else 0.0)) > 1e-3:
                    raise RuntimeError("exported rotation is not orthonormal")
        determinant = (rows[0][0] * (rows[1][1] * rows[2][2] - rows[1][2] * rows[2][1])
                       - rows[0][1] * (rows[1][0] * rows[2][2] - rows[1][2] * rows[2][0])
                       + rows[0][2] * (rows[1][0] * rows[2][1] - rows[1][1] * rows[2][0]))
        if abs(determinant - 1.0) > 1e-3:
            raise RuntimeError("exported rotation determinant differs from +1")
    posed_names = sorted(name for name, pose_id in zip(names, pose_ids) if pose_id in keyed_poses)
    if len(posed_names) != 46:
        raise RuntimeError("exported named pose count differs")
    return {"views": 48, "poses": 46, "intrinsics": len(intrinsics),
            "posed_names": posed_names, "missing_names": sorted(set(expected_names) - set(posed_names)),
            "structure": 0, "control_points": 0}


def save(output: Path, receipt: dict) -> None:
    (output / "receipt.json").write_text(json.dumps(receipt, sort_keys=True, indent=2) + "\n")


def run(output: Path = OUTPUT) -> dict:
    if sys.platform != "darwin" or resource is None:
        raise RuntimeError("OpenMVG native execution requires macOS resource limits")
    checked = preflight(output)
    output.mkdir()
    (output / "logs").mkdir()
    (output / "tmp").mkdir()
    receipt = {**checked, "status": "running", "started_utc": datetime.now(timezone.utc).isoformat()}
    save(output, receipt)
    started = time.monotonic()
    process = None
    log = output / "logs/01-convert.log"
    try:
        environment = os.environ.copy()
        for key in ("TMPDIR", "TMP", "TEMP", "XDG_CACHE_HOME"):
            environment[key] = str(output / "tmp")
        environment.update({"OMP_NUM_THREADS": "1", "OPENBLAS_NUM_THREADS": "1",
                            "VECLIB_MAXIMUM_THREADS": "1"})
        def child_limits() -> None:
            resource.setrlimit(resource.RLIMIT_CPU, (TIMEOUT, TIMEOUT))
            resource.setrlimit(resource.RLIMIT_FSIZE, (OUTPUT_CAP, OUTPUT_CAP))
        peak = 0
        with log.open("xb") as stream:
            process = subprocess.Popen(command(output), stdout=stream, stderr=subprocess.STDOUT,
                                       env=environment, start_new_session=True, preexec_fn=child_limits)
            while process.poll() is None:
                if time.monotonic() - started > TIMEOUT:
                    raise RuntimeError("pose export wall-time cap exceeded")
                if log.stat().st_size > LOG_CAP:
                    raise RuntimeError("pose export log cap exceeded")
                capacity(output)
                table = subprocess.check_output(["/bin/ps", "-axo", "pid=,ppid=,rss="], text=True)
                peak = max(peak, build.fifth.four.fork.base.process_rss_kib(process.pid, table))
                if peak > RSS_CAP_KIB:
                    raise RuntimeError("pose export process-tree RSS cap exceeded")
                time.sleep(0.1)
            if process.returncode or log.stat().st_size > LOG_CAP:
                raise RuntimeError(f"pose export exited {process.returncode} or exceeded log cap")
        capacity(output)
        if verify_failed_model() != checked["failed_model"] or verify_converter() != checked["converter"]:
            raise RuntimeError("sealed input/converter changed during export")
        receipt["exported"] = inspect_json(output / "sfm_camera_poses.json",
                                           json.loads((FAILED / "receipt.json").read_text())["photos"]["names"])
        receipt["status"] = "exported_failed_model_diagnostic_only"
    except BaseException as exc:
        receipt["status"] = "stopped"
        receipt["reason"] = str(exc)
        if process is not None and process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
        raise
    finally:
        receipt["finished_utc"] = datetime.now(timezone.utc).isoformat()
        receipt["wall_seconds"] = round(time.monotonic() - started, 3)
        receipt["returncode"] = process.returncode if process else None
        receipt["peak_rss_kib"] = locals().get("peak", 0)
        if log.exists() and log.stat().st_size > LOG_CAP:
            with log.open("r+b") as stream:
                stream.truncate(LOG_CAP)
            receipt["log_truncated_at_cap"] = True
        if log.exists():
            receipt["log_sha256"] = sha(log)
            receipt["log_bytes"] = log.stat().st_size
        exported = output / "sfm_camera_poses.json"
        if exported.exists() and exported.is_file() and not exported.is_symlink():
            receipt["json_sha256"] = sha(exported)
            receipt["json_bytes"] = exported.stat().st_size
        receipt["output_bytes"] = build.fifth.four.fork.base.tree_bytes(output)
        save(output, receipt)
    return receipt


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--convert", action="store_true", help="one-shot model conversion; requires review")
    args = parser.parse_args()
    print(json.dumps(run() if args.convert else preflight(), sort_keys=True, indent=2))


if __name__ == "__main__":
    main()
