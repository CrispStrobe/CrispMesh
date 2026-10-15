"""One-shot APFS-cloned 73-map OpenMVS bunny fusion and rough mesh.

Default invocation is a read-only preflight. The two failed source runs remain
untouched. This is a separately labeled cached-depth, geometric-iters=0 arm.
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

from scripts.classical_backend import openmvg_bunny_high_cache_rough as prior
from scripts.classical_backend.run import checked_ply, folder_bytes
from scripts.classical_backend.openmvg_photo_control import base as supervisor

SOURCE = Path("/Volumes/backups/code/crispmesh-data/openmvg-bunny-high-openmvs-cache-resume-002")
OUTPUT = Path("/Volumes/backups/code/crispmesh-data/openmvg-bunny-high-openmvs-cache-fusion-003")
PROBE = Path("/Volumes/backups/code/crispmesh-data/openmvg-bunny-clone-probe-001")
SOURCE_RESULT_SHA = "b559ef8970065b15cbf3df4a7796fef2275fd3b6831ea42321ddc1540933fd2e"
SCENE_SHA = "c93d436b1624c9f037bb8621ceccc6b5a9edd39f3e3b90d60877ce51a7b89510"
IMAGE_AGGREGATE_SHA = "2b66539cfd5757747fe12b626104a7236c60afa648b01b7c05a1e94392533148"
DMAP_AGGREGATE_SHA = "5ca1f3b7ac16bfa2f9ecb34bcc544efa113235b1f2c2c6beec2ceb21c1ea6263"
NAMES = tuple(f"frame_{i:04}.png" for i in range(73))
DMAP_NAMES = tuple(f"depth{i:04}.dmap" for i in range(73))
NATIVE_ARTIFACTS = ("dense.mvs", "dense.ply", "mesh.ply")
LOGICAL_CAP = 5 * (1 << 28)  # 1.25 GiB including COW-cloned inputs
PHYSICAL_CAP = 400 << 20
CLONE_PHYSICAL_CAP = 32 << 20
PER_CLONE_DELTA_CAP = 4 << 20
FLOOR = 11 << 30
LOG_CAP = 16 << 20
RSS_CAP_KIB = 4 << 20
TOTAL_SECONDS = 20 * 60
CACHE_SECONDS = 90


def sha(path: Path) -> str:
    return prior.sha(path)


def real_file(path: Path) -> Path:
    return prior.real_file(path)


def verify_clone_probe() -> dict:
    source = real_file(PROBE / "depth0000-clone-probe.dmap")
    original = real_file(SOURCE / "depth0000.dmap")
    if (source.stat().st_dev != original.stat().st_dev or
            source.stat().st_ino == original.stat().st_ino or
            source.stat().st_size != 10086207 or
            sha(source) != sha(original)):
        raise RuntimeError("APFS clone probe seal or distinct-inode check differs")
    info = subprocess.run(["/usr/sbin/diskutil", "info", "/Volumes/backups"],
                          capture_output=True, text=True, timeout=15, check=True).stdout
    if not re.search(r"^\s*File System Personality:\s*APFS\s*$", info, re.M):
        raise RuntimeError("external destination is not verified APFS")
    return {"source": str(original), "clone": str(source),
            "sha256": sha(source), "bytes": source.stat().st_size,
            "distinct_inodes": True, "same_device": True,
            "observed_probe_free_delta_bytes": 16384,
            "cp_c_fallback_risk": "macOS cp -c can fall back to physical copy; measured free-space limits enforce COW"}


def dmap_header(path: Path, index: int) -> dict:
    with path.open("rb") as stream:
        raw = stream.read(prior.DMAP_HEADER.size + 2)
        if len(raw) != prior.DMAP_HEADER.size + 2:
            raise RuntimeError(f"truncated DMAP header: {path.name}")
        magic, flags, _, image_w, image_h, width, height, _, _ = prior.DMAP_HEADER.unpack_from(raw)
        name_length = struct.unpack_from("<H", raw, prior.DMAP_HEADER.size)[0]
        name = stream.read(name_length).decode("utf-8", errors="strict")
    if (magic != 0x5244 or not flags & 1 or (image_w, image_h) != (874, 577) or
            (width, height) != (874, 577) or name != f"images/frame_{index:04}.png" or
            path.stat().st_size != 10086207):
        raise RuntimeError(f"cached base DMAP differs: {path.name}")
    return {"image": name, "dimensions": [width, height]}


def verify_source(source: Path = SOURCE) -> dict:
    if source.is_symlink() or not source.is_dir():
        raise RuntimeError("failed 73-base-map source missing")
    result_file, scene = real_file(source / "result.json"), real_file(source / "scene.mvs")
    if sha(result_file) != SOURCE_RESULT_SHA or sha(scene) != SCENE_SHA:
        raise RuntimeError("failed source result/scene seal differs")
    report = json.loads(result_file.read_text())
    if (report.get("schema") != "openmvg_bunny_high_cache_rough_v1" or
            report.get("status") != "failed" or
            report.get("failure") != "log or output cap exceeded" or
            report.get("source_unchanged") is not True or
            report.get("output_bytes", 0) <= prior.CAP or
            [(s.get("name"), s.get("status")) for s in report.get("stages", [])] !=
            [("densify", "failed")] or
            report.get("sealed_source", {}).get("result_sha256") != prior.SOURCE_RESULT_SHA):
        raise RuntimeError("failed 73-base-map source contract differs")
    image_dir = source / "images"
    if (image_dir.is_symlink() or not image_dir.is_dir() or
            {path.name for path in image_dir.iterdir()} != set(NAMES)):
        raise RuntimeError("73 image inventory differs")
    images = [image_dir / name for name in NAMES]
    image_aggregate, image_rows = prior.inventory(images)
    if image_aggregate != IMAGE_AGGREGATE_SHA:
        raise RuntimeError("73 image aggregate differs")
    maps = sorted(source.glob("depth[0-9][0-9][0-9][0-9].dmap"))
    if [path.name for path in maps] != list(DMAP_NAMES):
        raise RuntimeError("73 base DMAP inventory differs")
    headers = {path.name: dmap_header(path, i) for i, path in enumerate(maps)}
    dmap_aggregate, map_rows = prior.inventory(maps)
    if dmap_aggregate != DMAP_AGGREGATE_SHA:
        raise RuntimeError("73 base DMAP aggregate differs")
    recorded = report.get("output_inventory", {})
    if any(recorded.get(name, {}).get("sha256") != row["sha256"] for name, row in map_rows.items()):
        raise RuntimeError("failed source receipt does not bind all 73 base DMAPs")
    binary_rows = report["sealed_source"]["binary_sha256"]
    binaries = report["sealed_source"]["binaries"]
    for name in ("DensifyPointCloud", "ReconstructMesh"):
        if sha(real_file(Path(binaries[name]))) != binary_rows[name]:
            raise RuntimeError(f"native binary changed: {name}")
    return {"result_sha256": SOURCE_RESULT_SHA, "scene_sha256": SCENE_SHA,
            "images_aggregate_sha256": image_aggregate,
            "dmaps_aggregate_sha256": dmap_aggregate, "images": image_rows,
            "dmaps": map_rows, "dmap_headers": headers,
            "binaries": {name: binaries[name] for name in ("DensifyPointCloud", "ReconstructMesh")},
            "binary_sha256": {name: binary_rows[name] for name in
                              ("DensifyPointCloud", "ReconstructMesh")},
            "source_receipt_sha256": report["sealed_source"]["source_receipt_sha256"],
            "sparse_model_sha256": report["sealed_source"]["sparse_model_sha256"],
            "camera_gate_sha256": report["sealed_source"]["camera_gate_sha256"],
            "logical_clone_bytes": scene.stat().st_size +
                                   sum(row["bytes"] for row in image_rows.values()) +
                                   sum(row["bytes"] for row in map_rows.values())}


def capacity(output: Path, free_before: int, prospective: bool = False) -> dict:
    logical = folder_bytes(output)
    external = shutil.disk_usage(output.parent).free
    internal = shutil.disk_usage(Path(__file__).resolve().parents[2]).free
    physical_delta = max(0, free_before - external)
    if (logical > LOGICAL_CAP or physical_delta > PHYSICAL_CAP or
            external < FLOOR or internal < FLOOR or
            (prospective and (external < FLOOR + PHYSICAL_CAP or
                              logical > LOGICAL_CAP))):
        raise RuntimeError("logical cap, 400MiB physical increment, or 11GiB disk floor breached")
    return {"logical_output_bytes": logical, "external_free": external,
            "internal_free": internal, "physical_delta_bytes": physical_delta,
            "external_required_preflight": FLOOR + PHYSICAL_CAP}


def clone_one(source: Path, target: Path, expected: dict, free_before: int) -> int:
    source = real_file(source)
    if target.exists() or target.is_symlink():
        raise RuntimeError(f"clone destination already exists: {target.name}")
    before = shutil.disk_usage(target.parent).free
    subprocess.run(["/bin/cp", "-c", str(source), str(target)], check=True, timeout=30)
    target = real_file(target)
    after = shutil.disk_usage(target.parent).free
    if (source.stat().st_dev != target.stat().st_dev or
            source.stat().st_ino == target.stat().st_ino or
            target.stat().st_size != expected["bytes"] or
            sha(target) != expected["sha256"]):
        raise RuntimeError(f"clone inode/device/hash differs: {target.name}")
    per_delta = max(0, before - after)
    total_delta = max(0, free_before - after)
    if per_delta > PER_CLONE_DELTA_CAP or total_delta > CLONE_PHYSICAL_CAP:
        raise RuntimeError(f"cp -c physical-copy fallback suspected: {target.name}")
    if after < FLOOR:
        raise RuntimeError("11GiB floor breached during clone staging")
    return per_delta


def commands(output: Path, binaries: dict) -> list[tuple[str, list[str], str]]:
    common = ["--max-threads", "2", "--working-folder", str(output)]
    return [
        ("densify", [binaries["DensifyPointCloud"], "-i", str(output / "scene.mvs"),
                     "-o", str(output / "dense.mvs"), "--resolution-level", "3",
                     "--min-resolution", "640", "--max-resolution", "1280",
                     "--geometric-iters", "0", *common], "dense.ply"),
        ("mesh", [binaries["ReconstructMesh"], "-i", str(output / "dense.mvs"),
                  "-p", str(output / "dense.ply"), "-o", str(output / "mesh.mvs"),
                  *common], "mesh.ply"),
    ]


def preflight(output: Path = OUTPUT, source: Path = SOURCE) -> dict:
    if output.exists() or output.is_symlink():
        raise RuntimeError("one-shot fusion output must be fresh")
    if (not output.parent.is_dir() or output.parent.is_symlink() or
            output.parent.resolve() != SOURCE.parent.resolve()):
        raise RuntimeError("fusion output must be a fresh external sibling")
    probe = verify_clone_probe()
    sealed = verify_source(source)
    free_before = shutil.disk_usage(output.parent).free
    space = capacity(output, free_before, prospective=True)
    if sealed["logical_clone_bytes"] + (128 << 20) > LOGICAL_CAP:
        raise RuntimeError("logical cap leaves less than 128MiB for native rough outputs")
    return {"schema": "openmvg_bunny_high_cached73_fusion_rough_v1",
            "status": "read_only_preflight", "source": str(source), "output": str(output),
            "clone_probe": probe, "sealed_source": sealed, "capacity": space,
            "commands": {name: argv for name, argv, _ in commands(output, sealed["binaries"])},
            "limits": {"logical_output_bytes": LOGICAL_CAP,
                       "incremental_physical_bytes": PHYSICAL_CAP,
                       "clone_staging_physical_bytes": CLONE_PHYSICAL_CAP,
                       "disk_floor_bytes": FLOOR, "rss_kib": RSS_CAP_KIB,
                       "log_bytes_per_stage": LOG_CAP,
                       "real_wall_seconds_total": TOTAL_SECONDS,
                       "cache_reuse_seconds": CACHE_SECONDS, "threads_max": 2},
            "input_scope": "COW clone 73 images, scene, 73 complete base DMAPs; exclude 22 partial geo DMAPs",
            "role": "rough cached-fusion diagnostic; no geometric consistency, refinement, texture, or quality claim"}


def cache_fusion_checkpoint(output: Path, initial: dict, now_epoch: float,
                            stage_start_epoch: float) -> bool:
    log = (output / "densify.log").read_bytes()
    if b"Estimated depth-maps" in log or b"Geometric-consistent estimated depth-maps" in log:
        raise RuntimeError("native tool estimated depth maps instead of reusing 73 cached maps")
    for name, record in initial.items():
        stat = (output / name).stat()
        if (stat.st_size, stat.st_mtime_ns) != record:
            raise RuntimeError(f"cached base DMAP rewritten during fusion: {name}")
    if b"Dense fused depth-maps" in log or b"Depth-maps dense fused" in log:
        return True
    if now_epoch - stage_start_epoch > CACHE_SECONDS:
        raise RuntimeError("cached-fusion checkpoint not reached within 90 real seconds")
    return False


def run_stage(name: str, command: list[str], output: Path, started_epoch: float,
              free_before: int) -> dict:
    log = output / f"{name}.log"
    start = time.time()
    initial = {name: ((output / name).stat().st_size, (output / name).stat().st_mtime_ns)
               for name in DMAP_NAMES} if name == "densify" else {}
    cache_verified = name != "densify"
    peak = 0
    with log.open("xb") as stream:
        proc = subprocess.Popen(["/usr/bin/caffeinate", "-disu", *command], cwd=output,
                                stdout=stream, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            while proc.poll() is None:
                ps_elapsed = prior.elapsed_ps_seconds(proc.pid)
                if (time.time() - started_epoch > TOTAL_SECONDS or
                        (ps_elapsed is not None and ps_elapsed > TOTAL_SECONDS)):
                    raise RuntimeError("20-minute real wall watchdog exceeded")
                if log.stat().st_size > LOG_CAP:
                    raise RuntimeError("16MiB stage log cap exceeded")
                capacity(output, free_before)
                table = subprocess.check_output(["/bin/ps", "-axo", "pid=,ppid=,rss="], text=True)
                peak = max(peak, supervisor.process_rss_kib(proc.pid, table))
                if peak > RSS_CAP_KIB:
                    raise RuntimeError("4GiB process-tree RSS cap exceeded")
                if name == "densify" and not cache_verified:
                    cache_verified = cache_fusion_checkpoint(output, initial, time.time(), start)
                time.sleep(1)
            if proc.returncode != 0:
                raise RuntimeError(f"native stage returned {proc.returncode}")
            if name == "densify" and not cache_verified:
                cache_verified = cache_fusion_checkpoint(output, initial, time.time(), start)
            if not cache_verified:
                raise RuntimeError("densify ended without cache-fusion evidence")
            capacity(output, free_before)
        except BaseException:
            prior.stop_group(proc)
            raise
    return {"name": name, "status": "complete", "returncode": proc.returncode,
            "command": command, "caffeinate": "/usr/bin/caffeinate -disu",
            "real_wall_seconds": round(time.time() - start, 3),
            "peak_process_tree_rss_kib": peak, "log_sha256": sha(log),
            "log_bytes": log.stat().st_size, "cache_fusion_verified": cache_verified}


def finalize(report: dict, output: Path, sealed_source: dict,
             started_epoch: float, free_before: int) -> None:
    report["finished_epoch"] = time.time()
    report["real_wall_seconds_total"] = round(report["finished_epoch"] - started_epoch, 3)
    errors = []
    try:
        report["capacity_after"] = capacity(output, free_before)
        report["output_bytes"] = report["capacity_after"]["logical_output_bytes"]
    except Exception as exc:
        report["capacity_after_error"] = str(exc)
        report["output_bytes"] = folder_bytes(output)
        errors.append(f"capacity: {exc}")
    try:
        report["source_unchanged"] = verify_source() == sealed_source
        if not report["source_unchanged"]:
            errors.append("source changed")
    except Exception as exc:
        report["source_unchanged"] = False
        report["source_postcheck_error"] = str(exc)
        errors.append(f"source postcheck: {exc}")
    try:
        report["output_inventory"] = prior.output_inventory(output)
    except Exception as exc:
        report["inventory_error"] = str(exc)
        errors.append(f"inventory: {exc}")
    if report["real_wall_seconds_total"] > TOTAL_SECONDS:
        errors.append("20-minute real wall cap exceeded")
    if errors:
        report["status"] = "failed"
        report["postcheck_errors"] = errors
        report.setdefault("failure", "; ".join(errors))


def run(output: Path = OUTPUT) -> dict:
    checked = preflight(output)
    started_epoch = time.time()
    free_before = shutil.disk_usage(output.parent).free
    output.mkdir()
    (output / "images").mkdir()
    (output / "tmp").mkdir()
    report = {**checked, "status": "running", "stages": [],
              "started_epoch": started_epoch, "external_free_before": free_before,
              "quality_accepted": False, "metric_scale_verified": False,
              "shipping_approved": False}

    def save() -> None:
        (output / "result.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")

    save()
    try:
        source_rows = checked["sealed_source"]
        clone_deltas = {}
        clone_deltas["scene.mvs"] = clone_one(SOURCE / "scene.mvs", output / "scene.mvs",
                                               {"bytes": (SOURCE / "scene.mvs").stat().st_size,
                                                "sha256": SCENE_SHA}, free_before)
        for name, row in source_rows["images"].items():
            clone_deltas["images/" + name] = clone_one(SOURCE / "images" / name,
                                                       output / "images" / name, row, free_before)
            capacity(output, free_before)
        for name, row in source_rows["dmaps"].items():
            clone_deltas[name] = clone_one(SOURCE / name, output / name, row, free_before)
            capacity(output, free_before)
        if verify_source() != source_rows:
            raise RuntimeError("source changed during clone staging")
        if list(output.glob("depth*.geo.dmap")):
            raise RuntimeError("partial geometric DMAP copied into fusion arm")
        report["clone_staging"] = {"file_count": len(clone_deltas),
                                   "max_single_free_delta": max(clone_deltas.values()),
                                   "capacity": capacity(output, free_before)}
        save()
        for name, command, artifact_name in commands(output, source_rows["binaries"]):
            if time.time() - started_epoch > TOTAL_SECONDS:
                raise RuntimeError("20-minute real wall watchdog before stage")
            stage = {"name": name, "status": "running", "command": command}
            report["stages"].append(stage)
            save()
            try:
                stage.update(run_stage(name, command, output, started_epoch, free_before))
                artifact = real_file(output / artifact_name)
                count = checked_ply(artifact, "vertex" if name == "densify" else "face")
                stage["artifact"] = {"path": str(artifact), "bytes": artifact.stat().st_size,
                                     "sha256": sha(artifact),
                                     "points" if name == "densify" else "faces": count}
            except BaseException as exc:
                stage.update(status="failed", failure=str(exc))
                raise
            finally:
                save()
        if any(sha(output / name) != row["sha256"] for name, row in source_rows["dmaps"].items()):
            raise RuntimeError("cached 73 base DMAP hashes changed during fusion")
        report["artifacts"] = {name: {"bytes": (output / name).stat().st_size,
                                       "sha256": sha(output / name)}
                               for name in NATIVE_ARTIFACTS}
        report["status"] = "rough_complete_pending_quality_review"
    except BaseException as exc:
        report.update(status="failed", failure=str(exc))
    finalize(report, output, checked["sealed_source"], started_epoch, free_before)
    save()
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run-rough", action="store_true", help="one-shot live cached fusion")
    args = parser.parse_args()
    result = run() if args.run_rough else preflight()
    print(json.dumps({"status": result["status"], "output": result["output"],
                      "result": str(Path(result["output"]) / "result.json") if args.run_rough else None,
                      "failure": result.get("failure"),
                      "cached_depth_maps": len(result["sealed_source"]["dmaps"]),
                      "logical_clone_bytes": result["sealed_source"]["logical_clone_bytes"],
                      "capacity": result.get("capacity", result.get("capacity_after"))},
                     sort_keys=True))
    return 0 if result["status"] != "failed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
