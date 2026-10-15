"""One-shot mustard TRAIN SfM ablation with the photo-only intrinsic held fixed.

Default invocation is read-only. --run requires separate review; it reads the
sealed -001 photographs/features/matches and writes only to fresh -002 output.
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
import shutil
import signal
import subprocess
import sys
import time

from scripts.classical_backend import openmvg_mustard_photo_control as mustard
from scripts.classical_backend import openmvg_photo_control as core
from scripts.classical_backend import openmvg_sceaux_photo_control_v2 as cli

SOURCE = mustard.OUTPUT
OUTPUT = Path("/Volumes/backups/code/crispmesh-data/openmvg-mustard-photo-sfm-fixed-002")
SOURCE_RECEIPT_SHA = "e45c400dcf71a3ba86fa22f7af2138418d6dc511fddcd6d8470ef33599804744"
SFM_BINARY_SHA = "3d159212e82f16036b797337d3369ce6dfceabe17ff1745c48aaa2b545ab23f2"
INPUT_FILES = ("matches/sfm_data.json", "matches/image_describer.json", "matches/matches.e.bin")
OUTPUT_CAP = 256 << 20
LOG_CAP = 4 << 20
RSS_CAP_KIB = 1 << 20
WALL_SECONDS = 300
CPU_SECONDS = 600
DISK_FLOOR = 11 << 30


def command(output: Path = OUTPUT) -> list[str]:
    return [str(core.BIN_DIR / "openMVG_main_SfM"), "-i", str(output / "matches/sfm_data.json"),
            "-m", str(output / "matches"), "-M", "matches.e.bin", "-o", str(output / "sparse"),
            "-s", "INCREMENTAL", "-f", "NONE"]


def verify_inputs(source: Path = SOURCE) -> dict:
    receipt_path = source / "receipt.json"
    if (source.is_symlink() or not source.is_dir() or receipt_path.is_symlink() or
            not receipt_path.is_file() or core.sha(receipt_path) != SOURCE_RECEIPT_SHA):
        raise RuntimeError("sealed -001 source receipt differs")
    receipt = json.loads(receipt_path.read_text())
    if (receipt.get("status") != "failed_registration_or_sparse_gate" or
            receipt.get("sfm_report", {}).get("poses") != 46 or
            receipt.get("sfm_report", {}).get("views") != 48 or
            [stage.get("status") for stage in receipt.get("stages", [])] != ["completed"] * 5 or
            receipt["stages"][4].get("command", [])[-2:] != ["-f", "ADJUST_ALL"]):
        raise RuntimeError("-001 is not sealed failed ADJUST_ALL control")
    photos = receipt.get("photos", {})
    names = photos.get("names", [])
    hashes = photos.get("photo_sha256", {})
    if len(names) != 48 or len(set(names)) != 48 or set(hashes) != set(names):
        raise RuntimeError("-001 TRAIN photo inventory differs")
    inventory = receipt.get("output_inventory", {})
    if inventory != core.output_inventory(source):
        raise RuntimeError("full -001 source inventory differs from sealed receipt")
    required = set(INPUT_FILES) | {f"matches/{name[:-4]}.{suffix}"
                                   for name in names for suffix in ("feat", "desc")}
    needed = sorted(relative for relative in inventory if relative.startswith("matches/"))
    if len(needed) != 105 or not required <= set(needed) or \
            any(Path(relative).parent != Path("matches") for relative in needed):
        raise RuntimeError("sealed -001 flat match inventory differs")
    verified = {}
    for relative in needed:
        path = source / relative
        row = inventory.get(relative, {})
        if (path.is_symlink() or not path.is_file() or path.stat().st_size == 0 or
                row.get("bytes") != path.stat().st_size or core.sha(path) != row.get("sha256")):
            raise RuntimeError(f"sealed photo-derived input differs: {relative}")
        verified[relative] = row["sha256"]
    scene = json.loads((source / "matches/sfm_data.json").read_text())
    intrinsic = scene.get("intrinsics", [])
    if (scene.get("root_path") != str(source / "images") or len(scene.get("views", [])) != 48 or
            len(intrinsic) != 1 or scene.get("extrinsics") not in ([], {}) or
            scene.get("structure") not in ([], {}) or scene.get("control_points") not in ([], {})):
        raise RuntimeError("-001 listing scene is not photo-only 48-view input")
    inner = intrinsic[0]["value"]["ptr_wrapper"]["data"]
    if (inner.get("width"), inner.get("height"), inner.get("focal_length"),
            inner.get("principal_point"), inner.get("disto_k1")) != \
            (1280, 1024, 1536.0, [640.0, 512.0], [0.0]):
        raise RuntimeError("-001 image-only initial intrinsic differs")
    image_dir = source / "images"
    if image_dir.is_symlink() or {p.name for p in image_dir.iterdir()} != set(names):
        raise RuntimeError("-001 TRAIN image set differs")
    for name in names:
        item = image_dir / name
        if item.is_symlink() or not item.is_file() or core.sha(item) != hashes[name]:
            raise RuntimeError(f"sealed TRAIN photo differs: {name}")
    return {"source_receipt_sha256": SOURCE_RECEIPT_SHA, "reused_file_sha256": verified,
            "photo_sha256": hashes, "names": names, "initial_focal_px": 1536.0,
            "prior_report": receipt["sfm_report"]}


def capacity(output: Path, prospective: bool = False) -> dict:
    used = core.base.tree_bytes(output)
    external = core.base.free_bytes(output)
    internal = core.base.free_bytes(core.base.DEFAULT_INTERNAL)
    required = DISK_FLOOR + (OUTPUT_CAP - used if prospective else 0)
    if used > OUTPUT_CAP or external < required or internal < DISK_FLOOR:
        raise RuntimeError("fixed-intrinsic SfM disk/output cap breached")
    return {"output_bytes": used, "external_free": external,
            "external_required": required, "internal_free": internal}


def preflight(output: Path = OUTPUT) -> dict:
    if output.exists() or output.is_symlink():
        raise RuntimeError("one-shot fixed-intrinsic output must be fresh")
    if (not output.parent.is_dir() or output.parent.is_symlink() or
            output.parent.resolve() != SOURCE.parent.resolve()):
        raise RuntimeError("fixed-intrinsic output must be a fresh external sibling")
    inputs = verify_inputs()
    binaries = core.verify_binaries()
    if binaries["binary_sha256"].get("openMVG_main_SfM") != SFM_BINARY_SHA:
        raise RuntimeError("sealed SfM binary differs")
    argv = command(output)
    usage = cli.audit_cli_usage([("sfm", argv, str(output / "sparse/sfm_data.bin"))])
    return {"schema": "openmvg_mustard_fixed_intrinsic_sfm_v1",
            "status": "read_only_preflight", "source": str(SOURCE), "output": str(output),
            "inputs": inputs, "binaries": binaries, "compiled_cli_usage": usage,
            "command": argv, "capacity": capacity(output, prospective=True),
            "staged_match_files": sorted(inputs["reused_file_sha256"]),
            "limits": {"output_bytes": OUTPUT_CAP, "log_bytes": LOG_CAP,
                       "rss_kib": RSS_CAP_KIB, "wall_seconds": WALL_SECONDS,
                       "cpu_seconds": CPU_SECONDS, "disk_floor_bytes": DISK_FLOOR,
                       "threads_max": 2},
            "role": "photo-only fixed-intrinsic ablation; no reference inputs or quality claim",
            "license_scope": "evaluation only; no shipping or GPL/AGPL clearance"}


def save(output: Path, receipt: dict) -> None:
    core.save(output, receipt)


def stage_matches(output: Path, expected: dict[str, str], source: Path = SOURCE) -> dict[str, str]:
    staged = {}
    for relative, digest in expected.items():
        destination = output / relative
        shutil.copyfile(source / relative, destination)
        if core.sha(destination) != digest:
            raise RuntimeError(f"staged match input differs: {relative}")
        staged[relative] = digest
        capacity(output)
    verify_staged_matches(output, expected)
    return staged


def verify_staged_matches(output: Path, expected: dict[str, str]) -> None:
    if {f"matches/{path.name}" for path in (output / "matches").iterdir()} != set(expected):
        raise RuntimeError("staged match directory inventory differs")
    for relative, digest in expected.items():
        item = output / relative
        if item.is_symlink() or not item.is_file() or core.sha(item) != digest:
            raise RuntimeError(f"staged match file differs: {relative}")


def run(output: Path = OUTPUT) -> dict:
    if sys.platform != "darwin" or resource is None:
        raise RuntimeError("OpenMVG native execution requires macOS resource limits")
    checked = preflight(output)
    output.mkdir()
    for name in ("matches", "sparse", "logs", "tmp"):
        (output / name).mkdir()
    receipt = {**checked, "status": "running", "started_utc": datetime.now(timezone.utc).isoformat(),
               "stage": {"name": "sfm", "status": "pending", "command": command(output)}}
    save(output, receipt)
    started = time.monotonic()
    process = None
    peak = 0
    log = output / "logs/01-sfm.log"
    try:
        receipt["staged_match_sha256"] = stage_matches(output, checked["inputs"]["reused_file_sha256"])
        if verify_inputs() != checked["inputs"]:
            raise RuntimeError("sealed -001 inventory changed during match staging")
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
                    raise RuntimeError("SfM wall-time cap exceeded")
                if log.stat().st_size > LOG_CAP:
                    raise RuntimeError("SfM log cap exceeded")
                capacity(output)
                table = subprocess.check_output(["/bin/ps", "-axo", "pid=,ppid=,rss="], text=True)
                peak = max(peak, core.base.process_rss_kib(process.pid, table))
                if peak > RSS_CAP_KIB:
                    raise RuntimeError("SfM process-tree RSS cap exceeded")
                time.sleep(0.2)
            if process.returncode or log.stat().st_size > LOG_CAP:
                raise RuntimeError(f"SfM exited {process.returncode} or exceeded log cap")
        capacity(output)
        verify_staged_matches(output, checked["inputs"]["reused_file_sha256"])
        if verify_inputs() != checked["inputs"] or core.verify_binaries() != checked["binaries"]:
            raise RuntimeError("sealed SfM input/binary changed during ablation")
        model = output / "sparse/sfm_data.bin"
        if model.is_symlink() or not model.is_file() or model.stat().st_size == 0:
            raise RuntimeError("SfM model missing after successful exit")
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
    parser.add_argument("--run", action="store_true", help="one-shot live SfM; requires separate review")
    args = parser.parse_args()
    print(json.dumps(run() if args.run else preflight(), sort_keys=True, indent=2))


if __name__ == "__main__":
    main()
