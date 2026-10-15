"""Sealed one-shot rough OpenMVS continuation from 46 preserved bunny depth maps.

Default invocation is read-only preflight. --run-rough requires separate review.
The failed producer is never modified or used as a working directory.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import struct
import subprocess
import time

from scripts.classical_backend import openmvg_bunny_high_openmvs as prior
from scripts.classical_backend.run import checked_ply, folder_bytes
from scripts.classical_backend.openmvg_photo_control import base as supervisor

SOURCE = Path("/Volumes/backups/code/crispmesh-data/openmvg-bunny-high-openmvs-001")
OUTPUT = Path("/Volumes/backups/code/crispmesh-data/openmvg-bunny-high-openmvs-cache-resume-002")
SOURCE_RESULT_SHA = "4f70d6ef7859ef061a928a0101fd3be3a4da8eff7460629045a39ec4a4098acb"
SCENE_SHA = "c93d436b1624c9f037bb8621ceccc6b5a9edd39f3e3b90d60877ce51a7b89510"
IMAGE_AGGREGATE_SHA = "2b66539cfd5757747fe12b626104a7236c60afa648b01b7c05a1e94392533148"
DMAP_AGGREGATE_SHA = "9a9431a3a7ca3b185b4091e4559320fe174a73983959037ec8de18cc3969d31b"
NAMES = tuple(f"frame_{i:04}.png" for i in range(73))
DMAP_NAMES = tuple(f"depth{i:04}.dmap" for i in range(46))
CAP = 1 << 30
FLOOR = 11 << 30
LOG_CAP = 16 << 20
RSS_CAP_KIB = 4 << 20
TOTAL_SECONDS = 20 * 60
CACHE_SECONDS = 90
DMAP_HEADER = struct.Struct("<HBBIIIIff")


def sha(path: Path) -> str:
    return prior.sha256(path)


def real_file(path: Path) -> Path:
    if path.is_symlink() or not path.is_file():
        raise RuntimeError(f"missing or linked file: {path}")
    return path


def inventory(paths: list[Path]) -> tuple[str, dict]:
    rows = {}
    lines = []
    for path in paths:
        real_file(path)
        size, digest = path.stat().st_size, sha(path)
        rows[path.name] = {"bytes": size, "sha256": digest}
        lines.append(f"{path.name} {size} {digest}\n")
    return hashlib.sha256("".join(lines).encode()).hexdigest(), rows


def dmap_header(path: Path, index: int) -> dict:
    with path.open("rb") as stream:
        raw = stream.read(DMAP_HEADER.size + 2)
        if len(raw) != DMAP_HEADER.size + 2:
            raise RuntimeError(f"truncated cached DMAP: {path.name}")
        magic, flags, _, image_w, image_h, width, height, _, _ = DMAP_HEADER.unpack_from(raw)
        name_length = struct.unpack_from("<H", raw, DMAP_HEADER.size)[0]
        name = stream.read(name_length).decode("utf-8", errors="strict")
    if (magic != 0x5244 or not flags & 1 or (image_w, image_h) != (874, 577) or
            (width, height) != (874, 577) or name != f"images/frame_{index:04}.png" or
            path.stat().st_size != 10086207):
        raise RuntimeError(f"cached DMAP dimensions/name/size differ: {path.name}")
    return {"image": name, "dimensions": [width, height]}


def verify_source(source: Path = SOURCE) -> dict:
    if source.is_symlink() or not source.is_dir():
        raise RuntimeError("failed source run missing")
    result_file = real_file(source / "result.json")
    scene = real_file(source / "scene.mvs")
    if sha(result_file) != SOURCE_RESULT_SHA or sha(scene) != SCENE_SHA:
        raise RuntimeError("failed source result or scene seal differs")
    report = json.loads(result_file.read_text())
    if (report.get("schema") != "openmvg_bunny_high_openmvs_v1" or
            report.get("status") != "failed" or
            report.get("failure") != "densify: exit status -15" or
            report.get("source_unchanged") is not True or
            [(s.get("name"), s.get("status")) for s in report.get("stages", [])] !=
            [("convert", "complete"), ("densify", "failed")] or
            report["source"].get("source_receipt_sha256") !=
            "97cf72ab72c75a88f04d97baa29c6de49a1640103d7143dd151bc2efe54c507d" or
            report["source"].get("sparse_model_sha256") !=
            "aea2b3129cb317663435461fd12fa4c0078c57211fc1c08cf7b1a29ad6902975"):
        raise RuntimeError("failed source run contract differs")
    image_dir = source / "images"
    if (image_dir.is_symlink() or not image_dir.is_dir() or
            {p.name for p in image_dir.iterdir()} != set(NAMES)):
        raise RuntimeError("cached image inventory differs")
    images = [image_dir / name for name in NAMES]
    image_aggregate, image_rows = inventory(images)
    if image_aggregate != IMAGE_AGGREGATE_SHA:
        raise RuntimeError("cached image aggregate seal differs")
    maps = sorted(source.glob("depth[0-9][0-9][0-9][0-9].dmap"))
    if ([p.name for p in maps] != list(DMAP_NAMES) or
            list(source.glob("depth*.geo.dmap"))):
        raise RuntimeError("cached DMAP inventory differs")
    headers = {p.name: dmap_header(p, index) for index, p in enumerate(maps)}
    dmap_aggregate, dmap_rows = inventory(maps)
    if dmap_aggregate != DMAP_AGGREGATE_SHA:
        raise RuntimeError("cached DMAP aggregate seal differs")
    binary = real_file(Path(report["source"]["binaries"]["DensifyPointCloud"]))
    mesher = real_file(Path(report["source"]["binaries"]["ReconstructMesh"]))
    binary_hashes = report["source"]["binary_sha256"]
    if (sha(binary) != binary_hashes["DensifyPointCloud"] or
            sha(mesher) != binary_hashes["ReconstructMesh"]):
        raise RuntimeError("native OpenMVS binary seal differs")
    return {"result_sha256": SOURCE_RESULT_SHA, "scene_sha256": SCENE_SHA,
            "image_aggregate_sha256": image_aggregate, "dmap_aggregate_sha256": dmap_aggregate,
            "images": image_rows, "dmaps": dmap_rows, "dmap_headers": headers,
            "source_receipt_sha256": report["source"]["source_receipt_sha256"],
            "sparse_model_sha256": report["source"]["sparse_model_sha256"],
            "camera_gate_sha256": report["source"]["camera_gate_sha256"],
            "binaries": {"DensifyPointCloud": str(binary), "ReconstructMesh": str(mesher)},
            "binary_sha256": {key: binary_hashes[key] for key in
                              ("DensifyPointCloud", "ReconstructMesh")},
            "staged_bytes": scene.stat().st_size + sum(x["bytes"] for x in image_rows.values()) +
                            sum(x["bytes"] for x in dmap_rows.values())}


def capacity(output: Path, prospective: bool = False) -> dict:
    used = folder_bytes(output)
    external = shutil.disk_usage(output.parent).free
    internal = shutil.disk_usage(Path(__file__).resolve().parents[2]).free
    needed = FLOOR + (CAP if prospective else 0)
    if used > CAP or external < needed or internal < FLOOR:
        raise RuntimeError("output cap or 11 GiB disk reserve breached")
    return {"output_bytes": used, "external_free": external,
            "external_required": needed, "internal_free": internal}


def commands(output: Path, binaries: dict[str, str]) -> list[tuple[str, list[str], str]]:
    common = ["--max-threads", "2", "--working-folder", str(output)]
    return [
        ("densify", [binaries["DensifyPointCloud"], "-i", str(output / "scene.mvs"),
                     "-o", str(output / "dense.mvs"), "--resolution-level", "3",
                     "--min-resolution", "640", "--max-resolution", "1280",
                     "--geometric-iters", "2", *common], "dense.ply"),
        ("mesh", [binaries["ReconstructMesh"], "-i", str(output / "dense.mvs"),
                  "-p", str(output / "dense.ply"), "-o", str(output / "mesh.mvs"),
                  *common], "mesh.ply"),
    ]


def preflight(output: Path = OUTPUT, source: Path = SOURCE) -> dict:
    if output.exists() or output.is_symlink():
        raise RuntimeError("one-shot rough continuation output must be fresh")
    if (not output.parent.is_dir() or output.parent.is_symlink() or
            output.parent.resolve() != SOURCE.parent.resolve()):
        raise RuntimeError("rough continuation output must be external sibling")
    if not Path("/usr/bin/caffeinate").is_file():
        raise RuntimeError("macOS caffeinate unavailable")
    sealed = verify_source(source)
    if sealed["staged_bytes"] >= CAP:
        raise RuntimeError("cached inputs alone exceed output cap")
    space = capacity(output, prospective=True)
    stages = commands(output, sealed["binaries"])
    return {"schema": "openmvg_bunny_high_cache_rough_v1",
            "status": "read_only_preflight", "source": str(source), "output": str(output),
            "sealed_source": sealed, "capacity": space,
            "commands": {name: argv for name, argv, _ in stages},
            "limits": {"output_bytes": CAP, "disk_floor_bytes": FLOOR,
                       "rss_kib": RSS_CAP_KIB, "log_bytes_per_stage": LOG_CAP,
                       "real_wall_seconds_total": TOTAL_SECONDS,
                       "cache_reuse_seconds": CACHE_SECONDS, "threads_max": 2},
            "cache_contract": "46 copied complete base DMAPs; same scene/images/settings; early abort unless new depth0046 appears and first 46 remain byte-identical",
            "role": "rough mesh only; separate positive visual/geometry review before refinement or quality claim"}


def elapsed_ps_seconds(pid: int) -> int | None:
    result = subprocess.run(["/bin/ps", "-o", "etime=", "-p", str(pid)],
                            capture_output=True, text=True, timeout=5)
    value = result.stdout.strip()
    if not value:
        return None
    days = 0
    if "-" in value:
        day, value = value.split("-", 1)
        days = int(day)
    fields = [int(part) for part in value.split(":")]
    if len(fields) == 3:
        hours, minutes, seconds = fields
    elif len(fields) == 2:
        hours, minutes, seconds = 0, *fields
    else:
        raise RuntimeError("unexpected ps elapsed format")
    return days * 86400 + hours * 3600 + minutes * 60 + seconds


def stop_group(proc: subprocess.Popen) -> None:
    if proc.poll() is None:
        os.killpg(proc.pid, signal.SIGTERM)
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(proc.pid, signal.SIGKILL)
            proc.wait(timeout=5)


def cache_reuse_checkpoint(output: Path, cache_stats: dict, cache_rows: dict,
                           stage_start_epoch: float, now_epoch: float) -> bool:
    """Require the first new map before 90 seconds, with 46 cache files intact."""
    for key, initial in cache_stats.items():
        current = (output / key).stat()
        if (current.st_size, current.st_mtime_ns) != (initial.st_size, initial.st_mtime_ns):
            raise RuntimeError("cached DMAP was rewritten before reuse checkpoint")
    if (output / "depth0046.dmap").is_file():
        if any(sha(output / key) != row["sha256"] for key, row in cache_rows.items()):
            raise RuntimeError("cached DMAP hash changed before new map")
        return True
    if now_epoch - stage_start_epoch > CACHE_SECONDS:
        raise RuntimeError("cache reuse checkpoint not reached within 90 real seconds")
    return False


def run_stage(name: str, command: list[str], output: Path, started_epoch: float,
              cache_rows: dict) -> dict:
    log = output / f"{name}.log"
    start = time.time()
    cache_verified = name != "densify"
    cache_stats = {key: (output / key).stat() for key in cache_rows} if name == "densify" else {}
    peak = 0
    with log.open("xb") as stream:
        proc = subprocess.Popen(["/usr/bin/caffeinate", "-disu", *command], cwd=output,
                                stdout=stream, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            while proc.poll() is None:
                host_elapsed = time.time() - started_epoch
                ps_elapsed = elapsed_ps_seconds(proc.pid)
                if host_elapsed > TOTAL_SECONDS or (ps_elapsed is not None and ps_elapsed > TOTAL_SECONDS):
                    raise RuntimeError("20-minute real wall watchdog exceeded")
                if log.stat().st_size > LOG_CAP or folder_bytes(output) > CAP:
                    raise RuntimeError("log or output cap exceeded")
                capacity(output)
                table = subprocess.check_output(["/bin/ps", "-axo", "pid=,ppid=,rss="], text=True)
                peak = max(peak, supervisor.process_rss_kib(proc.pid, table))
                if peak > RSS_CAP_KIB:
                    raise RuntimeError("process-tree RSS cap exceeded")
                if name == "densify" and not cache_verified:
                    cache_verified = cache_reuse_checkpoint(output, cache_stats, cache_rows,
                                                            start, time.time())
                time.sleep(1)
            if proc.returncode != 0:
                raise RuntimeError(f"native stage returned {proc.returncode}")
            if name == "densify" and not cache_verified:
                raise RuntimeError("densify ended without cache reuse evidence")
            if log.stat().st_size > LOG_CAP or folder_bytes(output) > CAP:
                raise RuntimeError("post-stage log or output cap exceeded")
            capacity(output)
        except BaseException:
            stop_group(proc)
            raise
    return {"name": name, "status": "complete", "returncode": proc.returncode,
            "command": command, "caffeinate": "/usr/bin/caffeinate -disu",
            "real_wall_seconds": round(time.time() - start, 3),
            "peak_process_tree_rss_kib": peak, "log_sha256": sha(log),
            "log_bytes": log.stat().st_size, "cache_reuse_verified": cache_verified}


def output_inventory(output: Path) -> dict:
    rows = {}
    for directory, dirs, files in os.walk(output, followlinks=False):
        for name in dirs + files:
            path = Path(directory) / name
            if path.is_symlink():
                raise RuntimeError("symlink in rough continuation output")
            if path.is_file() and path != output / "result.json":
                rows[path.relative_to(output).as_posix()] = {
                    "bytes": path.stat().st_size, "sha256": sha(path)}
    return rows


def finalize(report: dict, output: Path, sealed_source: dict, started_epoch: float) -> None:
    """Record cap and postcheck failures even when the native run already stopped."""
    report["finished_epoch"] = time.time()
    report["real_wall_seconds_total"] = round(report["finished_epoch"] - started_epoch, 3)
    postcheck_errors = []
    try:
        report["output_bytes"] = folder_bytes(output)
    except Exception as exc:
        postcheck_errors.append(f"output size: {exc}")
    try:
        report["capacity_after"] = capacity(output)
    except Exception as exc:
        report["capacity_after_error"] = str(exc)
        postcheck_errors.append(f"capacity: {exc}")
    try:
        report["source_unchanged"] = verify_source() == sealed_source
        if not report["source_unchanged"]:
            postcheck_errors.append("source seal changed")
    except Exception as exc:
        report["source_unchanged"] = False
        report["source_postcheck_error"] = str(exc)
        postcheck_errors.append(f"source postcheck: {exc}")
    if report["real_wall_seconds_total"] > TOTAL_SECONDS:
        postcheck_errors.append("20-minute real wall cap exceeded")
    try:
        report["output_inventory"] = output_inventory(output)
    except Exception as exc:
        report["output_inventory_error"] = str(exc)
        postcheck_errors.append(f"inventory: {exc}")
    if postcheck_errors:
        report["status"] = "failed"
        report["postcheck_errors"] = postcheck_errors
        report.setdefault("failure", "; ".join(postcheck_errors))


def run(output: Path = OUTPUT) -> dict:
    checked = preflight(output)
    started_epoch = time.time()
    output.mkdir()
    (output / "images").mkdir()
    (output / "tmp").mkdir()
    report = {**checked, "status": "running", "stages": [],
              "started_epoch": started_epoch, "quality_accepted": False,
              "metric_scale_verified": False, "shipping_approved": False}

    def save() -> None:
        (output / "result.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")

    save()
    try:
        shutil.copy2(SOURCE / "scene.mvs", output / "scene.mvs")
        if sha(output / "scene.mvs") != SCENE_SHA:
            raise RuntimeError("staged scene differs")
        for name, row in checked["sealed_source"]["images"].items():
            shutil.copy2(SOURCE / "images" / name, output / "images" / name)
            if sha(output / "images" / name) != row["sha256"]:
                raise RuntimeError(f"staged image differs: {name}")
            capacity(output)
        for name, row in checked["sealed_source"]["dmaps"].items():
            shutil.copy2(SOURCE / name, output / name)
            if sha(output / name) != row["sha256"]:
                raise RuntimeError(f"staged DMAP differs: {name}")
            capacity(output)
        if verify_source() != checked["sealed_source"]:
            raise RuntimeError("source changed during cache staging")
        report["staged_bytes"] = folder_bytes(output)
        save()
        for name, command, artifact_name in commands(output, checked["sealed_source"]["binaries"]):
            if time.time() - started_epoch > TOTAL_SECONDS:
                raise RuntimeError("20-minute real wall watchdog before stage")
            stage = {"name": name, "status": "running", "command": command}
            report["stages"].append(stage)
            save()
            try:
                stage.update(run_stage(name, command, output, started_epoch,
                                       checked["sealed_source"]["dmaps"]))
                artifact = real_file(output / artifact_name)
                count = checked_ply(artifact, "vertex" if name == "densify" else "face")
                stage["artifact"] = {"path": str(artifact), "bytes": artifact.stat().st_size,
                                     "sha256": sha(artifact),
                                     "points" if name == "densify" else "faces": count}
                stage["status"] = "complete"
            except BaseException as exc:
                stage.update(status="failed", failure=str(exc))
                raise
            finally:
                save()
        report["artifacts"] = {name: {"bytes": (output / name).stat().st_size,
                                       "sha256": sha(output / name)}
                               for name in ("dense.mvs", "dense.ply", "mesh.mvs", "mesh.ply")}
        report["status"] = "rough_complete_pending_quality_review"
    except BaseException as exc:
        report.update(status="failed", failure=str(exc))
    finalize(report, output, checked["sealed_source"], started_epoch)
    save()
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run-rough", action="store_true", help="one-shot live cache continuation")
    args = parser.parse_args()
    result = run() if args.run_rough else preflight()
    print(json.dumps({"status": result["status"], "output": result["output"],
                      "result": str(Path(result["output"]) / "result.json") if args.run_rough else None,
                      "failure": result.get("failure"),
                      "cached_depth_maps": len(result["sealed_source"]["dmaps"]),
                      "staged_bytes": result["sealed_source"]["staged_bytes"],
                      "capacity": result.get("capacity", result.get("capacity_after"))},
                     sort_keys=True))
    return 0 if result["status"] != "failed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
