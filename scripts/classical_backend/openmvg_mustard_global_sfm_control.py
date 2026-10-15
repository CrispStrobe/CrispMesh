"""One-shot, photo-only OpenMVG GLOBAL SfM control on sealed mustard matches.

Default invocation is read-only. --run needs separate review; no Berkeley input.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
try:
    import resource
except ImportError:  # Windows test import; native execution is macOS-only.
    resource = None
import signal
import subprocess
import sys
import time

from scripts.classical_backend import openmvg_mustard_fixed_intrinsic_ablation as shared
from scripts.classical_backend import openmvg_mustard_photo_control as mustard
from scripts.classical_backend import openmvg_photo_control as core
from scripts.classical_backend import openmvg_sceaux_photo_control_v2 as cli

SOURCE = shared.SOURCE
OUTPUT = Path("/Volumes/backups/code/crispmesh-data/openmvg-mustard-photo-global-sfm-001")
SFM_BINARY_SHA = shared.SFM_BINARY_SHA
OUTPUT_CAP = shared.OUTPUT_CAP
LOG_CAP = shared.LOG_CAP
RSS_CAP_KIB = shared.RSS_CAP_KIB
WALL_SECONDS = 300
CPU_SECONDS = 600
DISK_FLOOR = shared.DISK_FLOOR
SOURCE_SEALS = {
    "src/software/SfM/main_SfM.cpp": "5d38ec108157fe52fc02980686b22ede24c5813ab47d304ff354b2e28f81c11f",
    "src/openMVG/sfm/pipelines/global/sfm_global_engine_relative_motions.cpp":
        "07261b630a977900fbb61eb7d6e59846ea3b9773a8e5327c2a6de590ed40e5af",
    "src/openMVG/sfm/pipelines/global/GlobalSfM_translation_averaging.hpp":
        "5900dff791d237eaa4454cf0ec6560bb21f3aa2b56d13e53b91cec802e5dd4f2",
}


def command(output: Path = OUTPUT) -> list[str]:
    return [str(core.BIN_DIR / "openMVG_main_SfM"), "-i", str(output / "matches/sfm_data.json"),
            "-m", str(output / "matches"), "-M", "matches.e.bin", "-o", str(output / "sparse"),
            "-s", "GLOBAL", "-f", "ADJUST_ALL"]


def capacity(output: Path, prospective: bool = False) -> dict:
    # The previous SfM-only control has the same external device, 256 MiB cap
    # and 11 GiB floor. Do not widen its resource envelope for GLOBAL.
    return shared.capacity(output, prospective)


def verify_source() -> dict:
    for relative, digest in SOURCE_SEALS.items():
        path = core.BUILD / "source" / relative
        if path.is_symlink() or not path.is_file() or core.sha(path) != digest:
            raise RuntimeError(f"reviewed GLOBAL source differs: {relative}")
    return SOURCE_SEALS.copy()


def preflight(output: Path = OUTPUT) -> dict:
    if output.exists() or output.is_symlink():
        raise RuntimeError("one-shot GLOBAL output must be fresh")
    if (not output.parent.is_dir() or output.parent.is_symlink() or
            output.parent.resolve() != SOURCE.parent.resolve()):
        raise RuntimeError("GLOBAL output must be a fresh external sibling")
    inputs = shared.verify_inputs()
    binaries = core.verify_binaries()
    source_seals = verify_source()
    if binaries["binary_sha256"].get("openMVG_main_SfM") != SFM_BINARY_SHA:
        raise RuntimeError("sealed SfM binary differs")
    argv = command(output)
    usage = cli.audit_cli_usage([("sfm_global", argv, str(output / "sparse/sfm_data.bin"))])
    return {"schema": "openmvg_mustard_global_sfm_control_v1",
            "status": "read_only_preflight", "source": str(SOURCE), "output": str(output),
            "inputs": inputs, "binaries": binaries, "compiled_cli_usage": usage,
            "reviewed_global_source_sha256": source_seals,
            "staged_match_files": sorted(inputs["reused_file_sha256"]),
            "command": argv, "capacity": capacity(output, prospective=True),
            "limits": {"output_bytes": OUTPUT_CAP, "log_bytes": LOG_CAP,
                       "rss_kib": RSS_CAP_KIB, "wall_seconds": WALL_SECONDS,
                       "cpu_seconds": CPU_SECONDS, "disk_floor_bytes": DISK_FLOOR,
                       "threads_max": 2},
            "role": "photo-only GLOBAL engine control; no GT input or quality claim",
            "license_scope": "evaluation only; no shipping or GPL/AGPL clearance"}


def save(output: Path, receipt: dict) -> None:
    core.save(output, receipt)


def run(output: Path = OUTPUT) -> dict:
    if sys.platform != "darwin" or resource is None:
        raise RuntimeError("native GLOBAL SfM requires macOS resource limits")
    checked = preflight(output)
    output.mkdir()
    for name in ("matches", "sparse", "logs", "tmp"):
        (output / name).mkdir()
    receipt = {**checked, "status": "running", "started_utc": datetime.now(timezone.utc).isoformat(),
               "stage": {"name": "sfm_global", "status": "pending", "command": command(output)}}
    save(output, receipt)
    started = time.monotonic()
    process = None
    peak = 0
    log = output / "logs/01-sfm-global.log"
    try:
        receipt["staged_match_sha256"] = shared.stage_matches(
            output, checked["inputs"]["reused_file_sha256"])
        if shared.verify_inputs() != checked["inputs"]:
            raise RuntimeError("sealed -001 inventory changed during GLOBAL staging")
        save(output, receipt)
        receipt["stage"]["status"] = "running"
        save(output, receipt)
        env = os.environ.copy()
        env.update({"OMP_NUM_THREADS": "2", "OPENBLAS_NUM_THREADS": "2",
                    "VECLIB_MAXIMUM_THREADS": "2", "TMPDIR": str(output / "tmp"),
                    "TMP": str(output / "tmp"), "TEMP": str(output / "tmp"),
                    "XDG_CACHE_HOME": str(output / "tmp")})
        def child_limits() -> None:
            resource.setrlimit(resource.RLIMIT_CPU, (CPU_SECONDS, CPU_SECONDS))
            resource.setrlimit(resource.RLIMIT_FSIZE, (OUTPUT_CAP, OUTPUT_CAP))
        with log.open("xb") as stream:
            process = subprocess.Popen(command(output), stdout=stream, stderr=subprocess.STDOUT,
                                       env=env, start_new_session=True, preexec_fn=child_limits)
            while process.poll() is None:
                if time.monotonic() - started > WALL_SECONDS:
                    raise RuntimeError("GLOBAL SfM wall-time cap exceeded")
                if log.stat().st_size > LOG_CAP:
                    raise RuntimeError("GLOBAL SfM log cap exceeded")
                capacity(output)
                table = subprocess.check_output(["/bin/ps", "-axo", "pid=,ppid=,rss="], text=True)
                peak = max(peak, core.base.process_rss_kib(process.pid, table))
                if peak > RSS_CAP_KIB:
                    raise RuntimeError("GLOBAL SfM process-tree RSS cap exceeded")
                time.sleep(0.2)
            if process.returncode or log.stat().st_size > LOG_CAP:
                raise RuntimeError(f"GLOBAL SfM exited {process.returncode} or exceeded log cap")
        capacity(output)
        shared.verify_staged_matches(output, checked["inputs"]["reused_file_sha256"])
        if (shared.verify_inputs() != checked["inputs"] or
                core.verify_binaries() != checked["binaries"] or
                verify_source() != checked["reviewed_global_source_sha256"]):
            raise RuntimeError("sealed GLOBAL input/binary changed during run")
        model = output / "sparse/sfm_data.bin"
        if model.is_symlink() or not model.is_file() or model.stat().st_size == 0:
            raise RuntimeError("GLOBAL SfM model missing after successful exit")
        receipt["stage"].update({"status": "completed", "model_sha256": core.sha(model)})
        receipt["sfm_report"] = mustard.report_counts(output / "sparse/SfMReconstruction_Report.html")
        report = receipt["sfm_report"]
        receipt["status"] = ("completed_pending_geometry_review" if report["poses"] == 48 and
                             report["tracks"] > 0 and report["residuals"] > 0 else
                             "failed_registration_or_sparse_gate")
    except BaseException as exc:
        receipt["status"] = "stopped"
        receipt["stage"]["status"] = "stopped"
        receipt["stage"]["reason"] = str(exc)
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
        receipt["stage"]["wall_seconds"] = round(time.monotonic() - started, 3)
        receipt["stage"]["returncode"] = process.returncode if process else None
        receipt["stage"]["peak_rss_kib"] = peak
        if log.exists() and log.stat().st_size > LOG_CAP:
            with log.open("r+b") as stream:
                stream.truncate(LOG_CAP)
            receipt["stage"]["log_truncated_at_cap"] = True
        if log.exists():
            receipt["stage"]["log_sha256"] = core.sha(log)
            receipt["stage"]["log_bytes"] = log.stat().st_size
        receipt["output_inventory"] = core.output_inventory(output)
        save(output, receipt)
    return receipt


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", action="store_true", help="one-shot native GLOBAL SfM; requires review")
    args = parser.parse_args()
    print(json.dumps(run() if args.run else preflight(), sort_keys=True, indent=2))


if __name__ == "__main__":
    main()
