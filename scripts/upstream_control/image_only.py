#!/usr/bin/env python3
"""Bounded stock PyCOLMAP sparse reconstruction of the pinned Sceaux JPEGs.

The upstream scene and meshes are verified as sample provenance only. The
reconstruction workers receive copied JPEGs and their own new database.
"""

from __future__ import annotations

import argparse
from contextlib import closing
import json
import math
from pathlib import Path
import platform
import shutil
import sqlite3
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from scripts.classical_backend.run import (RESERVE, StageError, digest, folder_bytes,
                                          stage)
from scripts.upstream_control.fetch import OUTPUT as SAMPLE
from scripts.upstream_control import fetch as sample_fetch
from scripts.upstream_control import run as sample_run
from scripts.upstream_control.run import verify_sample

PYTHON = ROOT / ".local-tools/colmap-sparse/venv/bin/python"
OUTPUT = Path("/Volumes/backups/code/crispmesh-data/sceaux-stock-sfm-001")
SEED = 20260927
MAX_BYTES = 512 << 20
MAX_RSS = 4 << 30
MAX_LOG = 16 << 20
MAX_SECONDS = 600
IMAGE_NAMES = tuple(f"{i:05}.jpg" for i in range(11))


def options(pycolmap, mask_path: str = ""):
    """Construct defaults anew; bound only the two CPU thread pools."""
    reader = pycolmap.ImageReaderOptions()
    if mask_path:
        reader.mask_path = mask_path
    extract = pycolmap.SiftExtractionOptions()
    extract.num_threads = 2
    matching = pycolmap.SiftMatchingOptions()
    matching.num_threads = 2
    exhaustive = pycolmap.ExhaustiveMatchingOptions()
    verification = pycolmap.TwoViewGeometryOptions()
    mapping = pycolmap.IncrementalPipelineOptions()
    mapping.num_threads = 2
    mapping.mapper.num_threads = 2
    return reader, extract, matching, exhaustive, verification, mapping


def effective_options(pycolmap, camera_mode=None, mask_path: str = ""):
    reader, extract, matching, exhaustive, verification, mapping = options(pycolmap, mask_path)
    camera_mode = camera_mode or pycolmap.CameraMode.AUTO
    deviations = {
        "random_seed": SEED,
        "device": "cpu",
        "sift_extraction.num_threads": 2,
        "sift_matching.num_threads": 2,
        "incremental_pipeline.num_threads": 2,
        "incremental_pipeline.mapper.num_threads": 2,
    }
    if camera_mode.name != "AUTO":
        deviations["camera_mode"] = camera_mode.name
    if mask_path:
        deviations["image_reader.mask_path"] = mask_path
    return {
        "camera_mode": camera_mode.name,
        "camera_model": reader.camera_model,
        "device": pycolmap.Device.cpu.name,
        "image_reader": reader.todict(),
        "sift_extraction": extract.todict(),
        "sift_matching": matching.todict(),
        "exhaustive_matching": exhaustive.todict(),
        "two_view_geometry": verification.todict(),
        "incremental_pipeline": mapping.todict(),
        "deviations_from_pycolmap_3_11_1_defaults": deviations,
    }


def inspect_model(model, names):
    registered_ids = set(model.reg_image_ids())
    registered = sorted(model.images[image_id].name for image_id in registered_ids)
    if len(registered) != len(set(registered)) or set(registered) - set(names):
        raise ValueError("model registers unexpected or duplicate image names")
    duplicate_tracks = 0
    duplicate_exact_observations = 0
    insufficient_distinct_views = 0
    invalid_track_links = 0
    orphan_point2d_links = 0
    nonfinite_observations = 0
    nonfinite_points = 0
    nonfinite_cameras = sum(
        not all(math.isfinite(float(v)) for v in camera.params)
        for camera in model.cameras.values())
    nonfinite_poses = sum(
        not all(math.isfinite(float(v)) for row in model.images[image_id].cam_from_world.matrix()
                for v in row)
        for image_id in registered_ids)
    track_links = set()
    for point_id, point in model.points3D.items():
        if not all(math.isfinite(float(v)) for v in point.xyz):
            nonfinite_points += 1
        elements = point.track.elements
        image_ids = [element.image_id for element in elements]
        if len(image_ids) != len(set(image_ids)):
            duplicate_tracks += 1
        if len(set(image_ids) & registered_ids) < 2:
            insufficient_distinct_views += 1
        seen = set()
        for element in elements:
            link = (element.image_id, element.point2D_idx)
            if link in seen:
                duplicate_exact_observations += 1
            seen.add(link)
            if element.image_id not in registered_ids or element.image_id not in model.images:
                invalid_track_links += 1
                continue
            image = model.images[element.image_id]
            if (not 0 <= element.point2D_idx < len(image.points2D) or
                    not image.points2D[element.point2D_idx].has_point3D() or
                    image.points2D[element.point2D_idx].point3D_id != point_id):
                invalid_track_links += 1
                continue
            track_links.add(link)
            if not all(math.isfinite(float(v)) for v in image.points2D[element.point2D_idx].xy):
                nonfinite_observations += 1
    for image_id in registered_ids:
        for index, observation in enumerate(model.images[image_id].points2D):
            if observation.has_point3D() and (
                    observation.point3D_id not in model.points3D or
                    (image_id, index) not in track_links):
                orphan_point2d_links += 1
    return {"registered_images": len(registered), "registered_names": registered,
            "missing_names": sorted(set(names) - set(registered)),
            "sparse_points": model.num_points3D(),
            "duplicate_image_tracks": duplicate_tracks,
            "duplicate_exact_observations": duplicate_exact_observations,
            "points_with_fewer_than_two_distinct_views": insufficient_distinct_views,
            "invalid_track_links": invalid_track_links,
            "orphan_point2d_links": orphan_point2d_links,
            "nonfinite_observations": nonfinite_observations,
            "nonfinite_points": nonfinite_points,
            "nonfinite_cameras": nonfinite_cameras,
            "nonfinite_poses": nonfinite_poses}


def worker(kind: str, output: Path) -> None:
    import pycolmap

    if pycolmap.__version__ != "3.11.1":
        raise ValueError(f"expected PyCOLMAP 3.11.1, got {pycolmap.__version__}")

    config_path = output / "worker-config.json"
    config = json.loads(config_path.read_text()) if config_path.is_file() else {}
    names = config.get("image_names", list(IMAGE_NAMES))
    if (not isinstance(names, list) or not names or len(names) != len(set(names)) or
            any(Path(name).name != name or name in (".", "..") for name in names)):
        raise ValueError("invalid worker image inventory")
    mode_name = config.get("camera_mode", "AUTO")
    if mode_name not in ("AUTO", "SINGLE"):
        raise ValueError("invalid worker camera mode")
    camera_mode = getattr(pycolmap.CameraMode, mode_name)
    mask_path = str(output / "masks") if config.get("use_masks", False) else ""
    images = output / "images"
    database = output / "database.db"
    reader, extract, matching, exhaustive, verification, mapping = options(pycolmap, mask_path)
    (output / "effective-options.json").write_text(
        json.dumps({"pycolmap_version": pycolmap.__version__,
                    "pycolmap_core_sha256": digest(Path(pycolmap._core.__file__)),
                    **effective_options(pycolmap, camera_mode, mask_path)}, indent=2, default=str) + "\n")
    pycolmap.set_random_seed(SEED)
    if kind == "features":
        pycolmap.extract_features(str(database), str(images), names,
                                  camera_mode=camera_mode,
                                  camera_model=reader.camera_model, reader_options=reader,
                                  sift_options=extract, device=pycolmap.Device.cpu)
    elif kind == "matching":
        pycolmap.match_exhaustive(str(database), sift_options=matching,
                                  matching_options=exhaustive,
                                  verification_options=verification,
                                  device=pycolmap.Device.cpu)
    elif kind == "mapping":
        models = pycolmap.incremental_mapping(str(database), str(images),
                                               str(output / "models"), options=mapping)
        summaries = []
        for index, model in sorted(models.items()):
            summary = {"index": index, **inspect_model(model, names)}
            directory = output / "models" / str(index)
            files = {name: directory / name for name in
                     ("cameras.bin", "images.bin", "points3D.bin")}
            if any(not path.is_file() or path.is_symlink() for path in files.values()):
                raise ValueError(f"incomplete binary model {index}")
            summary["files_sha256"] = {name: digest(path) for name, path in files.items()}
            summaries.append(summary)
        (output / "models.json").write_text(json.dumps(summaries, indent=2) + "\n")
    else:
        raise ValueError(f"unknown worker stage {kind}")


def database_counts(database: Path) -> dict:
    journal_files = [Path(str(database) + suffix) for suffix in ("-wal", "-shm", "-journal")]
    if any(path.exists() or path.is_symlink() for path in journal_files):
        raise ValueError("database has a live or leftover SQLite journal")
    with closing(sqlite3.connect(f"file:{database}?mode=ro&immutable=1", uri=True)) as connection:
        return {table: connection.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0]
                for table in ("images", "cameras", "keypoints", "descriptors",
                              "matches", "two_view_geometries")}


def partial_model_hashes(output: Path) -> dict:
    models = output / "models"
    if not models.is_dir() or models.is_symlink():
        return {}
    return {directory.name: {name: digest(path) for name in
                              ("cameras.bin", "images.bin", "points3D.bin")
                              if (path := directory / name).is_file() and not path.is_symlink()}
            for directory in sorted(models.iterdir())
            if directory.is_dir() and not directory.is_symlink()}


def toolchain(python: Path) -> dict:
    probe = subprocess.run([str(python), "-c",
                            "import json,pycolmap; print(json.dumps({'version':pycolmap.__version__,"
                            "'core':pycolmap._core.__file__}))"],
                           capture_output=True, text=True, timeout=10, check=True)
    info = json.loads(probe.stdout)
    if info["version"] != "3.11.1":
        raise ValueError(f"expected pinned PyCOLMAP 3.11.1, got {info['version']}")
    return {"version": info["version"], "core_path": info["core"],
            "core_sha256": digest(Path(info["core"])),
            "python_path": str(python),
            "runner_sha256": digest(Path(__file__)),
            "stage_helper_sha256": digest(ROOT / "scripts/classical_backend/run.py"),
            "sample_run_helper_sha256": digest(Path(sample_run.__file__)),
            "sample_fetch_helper_sha256": digest(Path(sample_fetch.__file__))}


def same_digest(path: Path, expected: str) -> bool:
    try:
        return digest(path) == expected
    except OSError:
        return False


def run(sample: Path, output: Path, python: Path = PYTHON) -> dict:
    sample = Path(sample)
    output = Path(output)
    if output.exists() or output.is_symlink():
        raise FileExistsError(f"output must be fresh: {output}")
    if not output.parent.is_dir() or output.parent.is_symlink():
        raise ValueError("output parent must be an existing real directory")
    output = output.absolute()
    if not python.is_file():
        raise ValueError(f"pinned PyCOLMAP interpreter unavailable: {python}")
    software = toolchain(python)
    sample_manifest, manifest_hash = verify_sample(sample)
    manifest_rows = {row["path"]: row for row in sample_manifest["files"]}
    if any(f"images/{name}" not in manifest_rows for name in IMAGE_NAMES):
        raise ValueError("Sceaux image manifest is incomplete")
    image_bytes = sum(manifest_rows[f"images/{name}"]["bytes"] for name in IMAGE_NAMES)
    if image_bytes >= MAX_BYTES:
        raise ValueError("source photos exceed output budget")
    if shutil.disk_usage(output.parent).free < RESERVE + MAX_BYTES:
        raise ValueError("output volume lacks 10 GiB reserve plus output budget")
    if shutil.disk_usage(ROOT).free < RESERVE:
        raise ValueError("workspace volume lacks 10 GiB reserve")
    output.mkdir()
    (output / "images").mkdir()
    deadline = time.monotonic() + MAX_SECONDS
    report = {"schema": "sceaux_image_only_pycolmap_stock_v2", "status": "running",
              "lane": "image-only sparse reconstruction; upstream scene/cameras/meshes excluded",
              "sample_manifest_sha256": manifest_hash,
              "sample_commit": sample_manifest["commit"],
              "software": software,
              "platform": platform.platform(), "machine": platform.machine(),
              "limits": {"max_output_bytes": MAX_BYTES, "max_child_rss_bytes": MAX_RSS,
                         "max_log_bytes_per_stage": MAX_LOG, "max_seconds_total": MAX_SECONDS,
                         "min_free_bytes_each_volume": RESERVE},
              "inputs": [], "stages": [], "models": [],
              "quality_accepted": False}

    def save():
        (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")

    save()
    try:
        for name in IMAGE_NAMES:
            source = sample / "images" / name
            target = output / "images" / name
            if (shutil.disk_usage(output).free < RESERVE + source.stat().st_size or
                    shutil.disk_usage(ROOT).free < RESERVE or
                    folder_bytes(output) + source.stat().st_size > MAX_BYTES):
                raise ValueError("image copy would exceed disk reserve or output cap")
            shutil.copyfile(source, target)
            checksum = manifest_rows[f"images/{name}"]["sha256"]
            if digest(target) != checksum:
                raise ValueError(f"copied image hash differs: {name}")
            report["inputs"].append({"name": name, "sha256": checksum,
                                     "bytes": source.stat().st_size})
        save()
        for kind in ("features", "matching", "mapping"):
            command = [str(python), "-m", "scripts.upstream_control.image_only",
                       "--worker", kind, "--output", str(output)]
            try:
                result = stage(output, kind, command, deadline, MAX_BYTES, MAX_LOG,
                               MAX_RSS, extra_reserve_paths=(ROOT,))
            except StageError as error:
                report["stages"].append(error.result)
                raise
            report["stages"].append(result)
            if kind == "features":
                report["effective_options"] = json.loads((output / "effective-options.json").read_text())
            if kind in ("features", "matching"):
                report[f"database_after_{kind}"] = {
                    "sha256": digest(output / "database.db"),
                    "counts": database_counts(output / "database.db")}
            save()
        report["models"] = json.loads((output / "models.json").read_text())
        report["database_after_mapping"] = {"sha256": digest(output / "database.db"),
                                            "counts": database_counts(output / "database.db")}
        best = max(report["models"], key=lambda m: (m["registered_images"], m["sparse_points"]),
                   default=None)
        report["best_model_index"] = best["index"] if best else None
        report["sparse_stage_eligible_not_quality"] = bool(
            best and best["registered_images"] == 11 and best["sparse_points"] > 0 and
            best["duplicate_exact_observations"] == 0 and
            best["points_with_fewer_than_two_distinct_views"] == 0 and
            best["invalid_track_links"] == 0 and best["orphan_point2d_links"] == 0 and
            best["nonfinite_observations"] == 0 and best["nonfinite_points"] == 0 and
            best["nonfinite_cameras"] == 0 and best["nonfinite_poses"] == 0)
        report["status"] = "complete"
    except Exception as error:
        report.update(status="failed", failure=str(error))
    finally:
        report["output_bytes"] = folder_bytes(output)
        report["source_images_unchanged"] = all(
            same_digest(sample / "images" / item["name"], item["sha256"])
            for item in report["inputs"])
        report["copied_images_unchanged"] = all(
            same_digest(output / "images" / item["name"], item["sha256"])
            for item in report["inputs"])
        report["free_bytes_after"] = {"output": shutil.disk_usage(output).free,
                                      "workspace": shutil.disk_usage(ROOT).free}
        try:
            report["software_unchanged"] = toolchain(python) == software
        except (OSError, ValueError, subprocess.SubprocessError):
            report["software_unchanged"] = False
        report["partial_model_files_sha256"] = partial_model_hashes(output)
        report["elapsed_seconds_total"] = round(MAX_SECONDS - (deadline - time.monotonic()), 3)
        violations = []
        if not report["source_images_unchanged"] or not report["copied_images_unchanged"]:
            violations.append("image input changed")
        if not report["software_unchanged"]:
            violations.append("software changed")
        if report["output_bytes"] > MAX_BYTES:
            violations.append("output byte limit exceeded")
        if any(free < RESERVE for free in report["free_bytes_after"].values()):
            violations.append("10 GiB disk reserve reached")
        if time.monotonic() > deadline:
            violations.append("total deadline exceeded")
        if violations:
            report.update(status="failed", failure="; ".join(violations),
                          sparse_stage_eligible_not_quality=False)
        save()
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sample", type=Path, default=SAMPLE)
    parser.add_argument("--output", type=Path, default=OUTPUT)
    parser.add_argument("--python", type=Path, default=PYTHON)
    parser.add_argument("--worker", choices=("features", "matching", "mapping"))
    args = parser.parse_args()
    if args.worker:
        worker(args.worker, args.output)
        return 0
    report = run(args.sample, args.output, args.python)
    print(json.dumps({"status": report["status"],
                      "sparse_stage_eligible_not_quality": report.get("sparse_stage_eligible_not_quality", False),
                      "report": str(args.output / "report.json")}))
    return 0 if report["status"] == "complete" else 1


if __name__ == "__main__":
    raise SystemExit(main())
