"""One-shot diagnostic JSON export of the sealed fixed-intrinsic mustard model.

Default invocation is read-only. --convert requires separate review.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
try:
    import resource
except ImportError:  # Windows test discovery; native execution is macOS-only.
    resource = None
import signal
import subprocess
import sys
import time

from scripts.classical_backend import openmvg_mustard_pose_export as prior
from scripts.classical_backend import openmvg_photo_control as core

FIXED = Path("/Volumes/backups/code/crispmesh-data/openmvg-mustard-photo-sfm-fixed-002")
OUTPUT = Path("/Volumes/backups/code/crispmesh-data/openmvg-mustard-photo-sfm-fixed-pose-export-003")
RECEIPT_SHA = "5445b512ac6d171e8097134d9c86d84e6428b1d958a3088593d6234e5bb1fa24"
MODEL_SHA = "4147cd6ea9e0848dd97210701535371c100425125dc83df07a67482ebd8eca55"
SFM_LOG_SHA = "7eaab977dddc6f1dc748fffc1d1c0affebf150bd762f56e333fdf9308898513d"
MODEL = FIXED / "sparse/sfm_data.bin"


def verify_fixed_model(fixed: Path = FIXED) -> dict:
    receipt_path = fixed / "receipt.json"
    model = fixed / "sparse/sfm_data.bin"
    log = fixed / "logs/01-sfm.log"
    if (fixed.is_symlink() or not fixed.is_dir() or
            any(item.is_symlink() or not item.is_file() for item in (receipt_path, model, log)) or
            prior.sha(receipt_path) != RECEIPT_SHA or prior.sha(model) != MODEL_SHA or
            prior.sha(log) != SFM_LOG_SHA):
        raise RuntimeError("sealed fixed-intrinsic receipt/model/log differs")
    receipt = json.loads(receipt_path.read_text())
    stage = receipt.get("stage", {})
    counts = receipt.get("sfm_report", {})
    if (receipt.get("schema") != "openmvg_mustard_fixed_intrinsic_sfm_v1" or
            receipt.get("status") != "failed_registration_or_sparse_gate" or
            (counts.get("views"), counts.get("poses"), counts.get("intrinsics"),
             counts.get("tracks"), counts.get("residuals")) != (48, 46, 1, 162, 1292) or
            stage.get("status") != "completed" or stage.get("returncode") != 0 or
            stage.get("model_sha256") != MODEL_SHA or stage.get("log_sha256") != SFM_LOG_SHA or
            stage.get("command", [])[-2:] != ["-f", "NONE"] or
            receipt.get("inputs", {}).get("source_receipt_sha256") != prior.FAIL_RECEIPT_SHA or
            receipt.get("inputs", {}).get("initial_focal_px") != 1536.0 or
            receipt.get("output_inventory", {}).get("sparse/sfm_data.bin", {}).get("sha256") != MODEL_SHA or
            receipt.get("output_inventory") != core.output_inventory(fixed)):
        raise RuntimeError("sealed fixed-intrinsic run contract differs")
    names = receipt["inputs"].get("names", [])
    if len(names) != 48 or len(set(names)) != 48:
        raise RuntimeError("fixed-intrinsic TRAIN names differ")
    return {"receipt_sha256": RECEIPT_SHA, "model_sha256": MODEL_SHA,
            "sfm_log_sha256": SFM_LOG_SHA, "sfm_report": counts, "names": names,
            "status": receipt["status"]}


def command(output: Path = OUTPUT) -> list[str]:
    return [str(prior.build.BINARY), "-i", str(MODEL),
            "-o", str(output / "sfm_camera_poses.json"), "-V", "-I", "-E"]


def preflight(output: Path = OUTPUT) -> dict:
    if output.exists() or output.is_symlink():
        raise RuntimeError("one-shot fixed-intrinsic export output must be fresh")
    if (not output.parent.is_dir() or output.parent.is_symlink() or
            output.parent.resolve() != FIXED.parent.resolve()):
        raise RuntimeError("fixed-intrinsic export must be a fresh external sibling")
    fixed = verify_fixed_model()
    converter = prior.verify_converter()
    return {"schema": "openmvg_mustard_fixed_pose_export_v1", "status": "read_only_preflight",
            "fixed_model": fixed, "converter": converter, "output": str(output),
            "command": command(output), "capacity": prior.capacity(output, prospective=True),
            "limits": {"output_bytes": prior.OUTPUT_CAP, "log_bytes": prior.LOG_CAP,
                       "rss_kib": prior.RSS_CAP_KIB, "wall_seconds": prior.TIMEOUT,
                       "cpu_seconds": prior.TIMEOUT, "disk_floor_bytes": prior.FLOOR},
            "role": "diagnostic export of fixed-intrinsic 46/48 model; no reference input",
            "license_scope": "evaluation only; no shipping or GPL/AGPL clearance"}


def run(output: Path = OUTPUT) -> dict:
    if sys.platform != "darwin" or resource is None:
        raise RuntimeError("OpenMVG native execution requires macOS resource limits")
    checked = preflight(output)
    output.mkdir()
    (output / "logs").mkdir()
    (output / "tmp").mkdir()
    receipt = {**checked, "status": "running", "started_utc": datetime.now(timezone.utc).isoformat()}
    prior.save(output, receipt)
    started = time.monotonic()
    process = None
    peak = 0
    log = output / "logs/01-convert.log"
    try:
        env = os.environ.copy()
        for key in ("TMPDIR", "TMP", "TEMP", "XDG_CACHE_HOME"):
            env[key] = str(output / "tmp")
        env.update({"OMP_NUM_THREADS": "1", "OPENBLAS_NUM_THREADS": "1",
                    "VECLIB_MAXIMUM_THREADS": "1"})
        def child_limits() -> None:
            resource.setrlimit(resource.RLIMIT_CPU, (prior.TIMEOUT, prior.TIMEOUT))
            resource.setrlimit(resource.RLIMIT_FSIZE, (prior.OUTPUT_CAP, prior.OUTPUT_CAP))
        with log.open("xb") as stream:
            process = subprocess.Popen(command(output), stdout=stream, stderr=subprocess.STDOUT,
                                       env=env, start_new_session=True, preexec_fn=child_limits)
            while process.poll() is None:
                if time.monotonic() - started > prior.TIMEOUT:
                    raise RuntimeError("fixed-intrinsic export wall-time cap exceeded")
                if log.stat().st_size > prior.LOG_CAP:
                    raise RuntimeError("fixed-intrinsic export log cap exceeded")
                prior.capacity(output)
                table = subprocess.check_output(["/bin/ps", "-axo", "pid=,ppid=,rss="], text=True)
                peak = max(peak, prior.build.fifth.four.fork.base.process_rss_kib(process.pid, table))
                if peak > prior.RSS_CAP_KIB:
                    raise RuntimeError("fixed-intrinsic export process-tree RSS cap exceeded")
                time.sleep(0.1)
            if process.returncode or log.stat().st_size > prior.LOG_CAP:
                raise RuntimeError(f"fixed-intrinsic export exited {process.returncode} or exceeded log cap")
        prior.capacity(output)
        if verify_fixed_model() != checked["fixed_model"] or prior.verify_converter() != checked["converter"]:
            raise RuntimeError("sealed fixed model/converter changed during export")
        receipt["exported"] = prior.inspect_json(output / "sfm_camera_poses.json",
                                                   checked["fixed_model"]["names"])
        receipt["status"] = "exported_fixed_model_diagnostic_only"
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
        receipt["peak_rss_kib"] = peak
        if log.exists() and log.stat().st_size > prior.LOG_CAP:
            with log.open("r+b") as stream:
                stream.truncate(prior.LOG_CAP)
            receipt["log_truncated_at_cap"] = True
        if log.exists():
            receipt["log_sha256"] = prior.sha(log)
            receipt["log_bytes"] = log.stat().st_size
        exported = output / "sfm_camera_poses.json"
        if exported.exists() and exported.is_file() and not exported.is_symlink():
            receipt["json_sha256"] = prior.sha(exported)
            receipt["json_bytes"] = exported.stat().st_size
        receipt["output_bytes"] = prior.build.fifth.four.fork.base.tree_bytes(output)
        prior.save(output, receipt)
    return receipt


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--convert", action="store_true", help="one-shot native conversion; requires review")
    args = parser.parse_args()
    print(json.dumps(run() if args.convert else preflight(), sort_keys=True, indent=2))


if __name__ == "__main__":
    main()
