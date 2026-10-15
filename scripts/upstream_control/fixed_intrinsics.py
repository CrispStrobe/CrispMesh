#!/usr/bin/env python3
"""One bounded mapping-only fixed-initial-intrinsics cracker-box diagnostic."""

from __future__ import annotations

import argparse
from contextlib import closing
import json
from pathlib import Path
import platform
import shutil
import sqlite3
import struct
import time

from scripts.classical_backend.run import RESERVE, StageError, digest, folder_bytes, stage
from scripts.upstream_control import image_only as stock
from scripts.upstream_control import object_sparse

ROOT = stock.ROOT
SOURCE = Path("/Volumes/backups/code/crispmesh-data/cracker-stock-masked-001")
OUTPUT_PARENT = Path("/Volumes/backups/code/crispmesh-data")
PYTHON = stock.PYTHON
NAMES = object_sparse.NAMES
EXPECTED_DB_SHA256 = "a91caa5b6ae7c3c50f895fe682a006190118846d0510e0c8fa7ed0a1c47d9ae7"
INITIAL_CAMERA = (1536.0, 640.0, 512.0, 0.0)
STRUCTURAL_KEYS = object_sparse.STRUCTURAL_KEYS
ARMS = ("fixed-automatic", "native-seed30")
SEED_NAMES = ("NP3_192.jpg", "NP3_162.jpg")


def regular(path: Path) -> None:
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"expected regular file: {path}")


def camera_rows(database: Path) -> list[dict]:
    with closing(sqlite3.connect(f"file:{database}?mode=ro&immutable=1", uri=True)) as connection:
        rows = connection.execute("SELECT camera_id, model, width, height, params, prior_focal_length "
                                  "FROM cameras ORDER BY camera_id").fetchall()
    return [{"camera_id": row[0], "model_id": row[1], "width": row[2], "height": row[3],
             "params": list(struct.unpack(f"<{len(row[4]) // 8}d", row[4])),
             "prior_focal_length": row[5]} for row in rows]


def database_counts(database: Path) -> dict:
    with closing(sqlite3.connect(f"file:{database}?mode=ro&immutable=1", uri=True)) as connection:
        return {table: connection.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0]
                for table in ("images", "cameras", "keypoints", "descriptors",
                              "matches", "two_view_geometries")}


def source_inventory(source: Path) -> tuple[list[dict], dict]:
    regular(source / "database.db")
    for suffix in ("-wal", "-journal"):
        journal = Path(str(source / "database.db") + suffix)
        if journal.exists() and journal.stat().st_size:
            raise ValueError(f"source database has a nonempty SQLite journal: {journal}")
    db_hash = digest(source / "database.db")
    if db_hash != EXPECTED_DB_SHA256:
        raise ValueError("source database differs from sealed mapping baseline")
    regular(source / "report.json")
    original = json.loads((source / "report.json").read_text())
    if (original.get("arm") != "masked" or original.get("status") != "complete" or
            original.get("database_after_mapping", {}).get("sha256") != db_hash or
            original.get("effective_options", {}).get("pycolmap_version") != "3.11.1"):
        raise ValueError("source report does not bind the sealed masked database")
    images = original.get("inputs", [])
    if [item.get("name") for item in images] != list(NAMES):
        raise ValueError("source report image inventory differs from 60 NP3 views")
    for item in images:
        path = source / "images" / item["name"]
        regular(path)
        if path.stat().st_size != item["bytes"] or digest(path) != item["sha256"]:
            raise ValueError(f"source image differs from sealed report: {item['name']}")
    counts = database_counts(source / "database.db")
    if counts != original["database_after_mapping"]["counts"]:
        raise ValueError("source database counts differ from sealed report")
    cameras = camera_rows(source / "database.db")
    if (len(cameras) != 1 or cameras[0]["model_id"] != 2 or
            cameras[0]["params"] != list(INITIAL_CAMERA) or
            cameras[0]["width"] != 1280 or cameras[0]["height"] != 1024):
        raise ValueError("source database does not contain the expected initial camera")
    with closing(sqlite3.connect(f"file:{source / 'database.db'}?mode=ro&immutable=1", uri=True)) as connection:
        names = [row[0] for row in connection.execute("SELECT name FROM images ORDER BY name")]
    if names != sorted(NAMES):
        raise ValueError("source database image names differ from 60 NP3 views")
    return images, {"sha256": db_hash, "counts": counts, "cameras": cameras,
                    "source_report_sha256": digest(source / "report.json")}


def seed_ids(database: Path) -> tuple[int, int]:
    with closing(sqlite3.connect(f"file:{database}?mode=ro&immutable=1", uri=True)) as connection:
        rows = connection.execute("SELECT image_id, name FROM images WHERE name IN (?, ?)",
                                  SEED_NAMES).fetchall()
    by_name = {name: image_id for image_id, name in rows}
    if set(by_name) != set(SEED_NAMES):
        raise ValueError("30-degree seed pair absent from copied database")
    ids = tuple(by_name[name] for name in SEED_NAMES)
    if ids != (33, 28):
        raise ValueError(f"30-degree seed pair ID mapping changed: {ids}")
    return ids


def mapping_options(pycolmap, arm: str, pair_ids: tuple[int, int] | None = None):
    if arm not in ARMS:
        raise ValueError(f"unknown diagnostic arm: {arm}")
    options = pycolmap.IncrementalPipelineOptions()
    options.num_threads = 2
    options.mapper.num_threads = 2
    if arm == "fixed-automatic":
        if pair_ids is not None:
            raise ValueError("fixed-automatic arm must use automatic initial pair")
        options.ba_refine_focal_length = False
        options.ba_refine_principal_point = False
        options.ba_refine_extra_params = False
        options.mapper.abs_pose_refine_focal_length = False
        options.mapper.abs_pose_refine_extra_params = False
        if options.init_image_id1 != -1 or options.init_image_id2 != -1:
            raise ValueError("mapper must select its own initial image pair")
    else:
        if pair_ids != (33, 28):
            raise ValueError("native-seed30 requires verified NP3_192/162 IDs")
        options.init_image_id1, options.init_image_id2 = pair_ids
    return options


def worker(output: Path) -> None:
    import pycolmap

    if pycolmap.__version__ != "3.11.1":
        raise ValueError(f"expected PyCOLMAP 3.11.1, got {pycolmap.__version__}")
    config = json.loads((output / "worker-config.json").read_text())
    arm = config["arm"]
    pair_ids = seed_ids(output / "database.db") if arm == "native-seed30" else None
    options = mapping_options(pycolmap, arm, pair_ids)
    changes = ({"ba_refine_focal_length": False, "ba_refine_principal_point": False,
                "ba_refine_extra_params": False, "mapper.abs_pose_refine_focal_length": False,
                "mapper.abs_pose_refine_extra_params": False}
               if arm == "fixed-automatic" else
               {"init_image_id1": pair_ids[0], "init_image_id2": pair_ids[1]})
    (output / "effective-options.json").write_text(json.dumps({
        "pycolmap_version": pycolmap.__version__,
        "pycolmap_core_sha256": digest(Path(pycolmap._core.__file__)),
        "random_seed": stock.SEED,
        "arm": arm,
        "initial_pair_policy": ("automatic" if arm == "fixed-automatic" else
                                {"names": SEED_NAMES, "image_ids": pair_ids}),
        "intrinsics_policy": ("fixed initial database camera in BA and absolute pose"
                              if arm == "fixed-automatic" else "stock native refinement"),
        "incremental_pipeline": options.todict(),
        "changes_from_stock_mapping": changes}, indent=2, default=str) + "\n")
    pycolmap.set_random_seed(stock.SEED)
    models = pycolmap.incremental_mapping(str(output / "database.db"), str(output / "images"),
                                          str(output / "models"), options=options)
    summaries = []
    for index, model in sorted(models.items()):
        summary = {"index": index, **stock.inspect_model(model, NAMES)}
        summary["cameras"] = [{"camera_id": camera_id, "model": str(camera.model),
                               "width": camera.width, "height": camera.height,
                               "params": [float(value) for value in camera.params]}
                              for camera_id, camera in sorted(model.cameras.items())]
        directory = output / "models" / str(index)
        files = {name: directory / name for name in ("cameras.bin", "images.bin", "points3D.bin")}
        if any(not path.is_file() or path.is_symlink() for path in files.values()):
            raise ValueError(f"incomplete binary model {index}")
        summary["files_sha256"] = {name: digest(path) for name, path in files.items()}
        summaries.append(summary)
    (output / "models.json").write_text(json.dumps(summaries, indent=2) + "\n")


def run(source: Path, output: Path, arm: str, python: Path = PYTHON) -> dict:
    source, output, python = Path(source), Path(output), Path(python)
    if arm not in ARMS:
        raise ValueError(f"unknown diagnostic arm: {arm}")
    if output.exists() or output.is_symlink():
        raise FileExistsError(f"output must be fresh: {output}")
    if output.parent.is_symlink() or not output.parent.is_dir():
        raise ValueError("output parent must be an existing real directory")
    if output.parent.resolve() != OUTPUT_PARENT.resolve():
        raise ValueError(f"output must be a fresh directory under {OUTPUT_PARENT}")
    if not python.is_file():
        raise ValueError(f"pinned PyCOLMAP interpreter unavailable: {python}")
    images, database = source_inventory(source)
    software = stock.toolchain(python)
    software["fixed_intrinsics_runner_sha256"] = digest(Path(__file__))
    if (shutil.disk_usage(output.parent).free < RESERVE + stock.MAX_BYTES or
            shutil.disk_usage(ROOT).free < RESERVE):
        raise ValueError("10 GiB volume reserve or output budget unavailable")
    output = output.absolute()
    output.mkdir()
    (output / "images").mkdir()
    deadline = time.monotonic() + stock.MAX_SECONDS
    report = {
        "schema": "cracker_masked_mapping_diagnostic_v1", "status": "running", "arm": arm,
        "source": str(source), "source_database": database,
        "source_image_inventory": images, "software": software,
        "platform": platform.platform(), "machine": platform.machine(),
        "limits": {"max_output_bytes": stock.MAX_BYTES, "max_child_rss_bytes": stock.MAX_RSS,
                   "max_log_bytes": stock.MAX_LOG, "max_seconds_total": stock.MAX_SECONDS,
                   "min_free_bytes_each_volume": RESERVE},
        "mapping_only": True, "ground_truth_inputs": False,
        "models": [], "stages": [], "quality_accepted": False,
        "independent_camera_review_complete": False}

    def save() -> None:
        (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")

    save()
    try:
        for item in images:
            src = source / "images" / item["name"]
            target = output / "images" / item["name"]
            if (folder_bytes(output) + item["bytes"] > stock.MAX_BYTES or
                    shutil.disk_usage(output).free < RESERVE + item["bytes"] or
                    shutil.disk_usage(ROOT).free < RESERVE):
                raise ValueError("image copy exceeds output cap or disk reserve")
            shutil.copyfile(src, target)
            if digest(target) != item["sha256"]:
                raise ValueError(f"copied image hash differs: {item['name']}")
        source_db = source / "database.db"
        copied_db = output / "database.db"
        if (folder_bytes(output) + source_db.stat().st_size > stock.MAX_BYTES or
                shutil.disk_usage(output).free < RESERVE + source_db.stat().st_size):
            raise ValueError("database copy exceeds output cap or disk reserve")
        shutil.copyfile(source_db, copied_db)
        if digest(copied_db) != database["sha256"]:
            raise ValueError("copied database hash differs from source")
        report["copied_database_before_mapping"] = {"sha256": digest(copied_db),
                                                     "counts": database_counts(copied_db),
                                                     "cameras": camera_rows(copied_db)}
        (output / "worker-config.json").write_text(json.dumps({"arm": arm}) + "\n")
        report["worker_config_sha256"] = digest(output / "worker-config.json")
        if arm == "native-seed30":
            report["verified_seed_pair"] = {"names": SEED_NAMES, "image_ids": seed_ids(copied_db)}
        save()
        command = [str(python), "-m", "scripts.upstream_control.fixed_intrinsics",
                   "--worker", "--arm", arm, "--output", str(output)]
        try:
            result = stage(output, "mapping", command, deadline, stock.MAX_BYTES,
                           stock.MAX_LOG, stock.MAX_RSS, extra_reserve_paths=(ROOT,))
        except StageError as error:
            report["stages"].append(error.result)
            raise
        report["stages"].append(result)
        report["effective_options"] = json.loads((output / "effective-options.json").read_text())
        report["models"] = json.loads((output / "models.json").read_text())
        report["copied_database_after_mapping"] = {"sha256": digest(copied_db),
                                                    "counts": database_counts(copied_db),
                                                    "cameras": camera_rows(copied_db)}
        for model in report["models"]:
            model["camera_exact_initial"] = bool(model["cameras"] and all(
                camera["params"] == list(INITIAL_CAMERA) for camera in model["cameras"]))
        best = max(report["models"], key=lambda model: (model["registered_images"],
                                                        model["sparse_points"], -model["index"]),
                   default=None)
        report["best_model_index"] = best["index"] if best else None
        report["third_view_registered"] = bool(best and best["registered_images"] >= 3)
        report["structural_sparse_eligible_pending_camera_review"] = bool(
            best and best["registered_images"] >= 54 and best["sparse_points"] > 0 and
            (arm != "fixed-automatic" or best["camera_exact_initial"]) and
            all(best[key] == 0 for key in STRUCTURAL_KEYS))
        report["status"] = "complete"
    except Exception as error:
        report.update(status="failed", failure=str(error),
                      structural_sparse_eligible_pending_camera_review=False)
    finally:
        copied_db = output / "database.db"
        if copied_db.is_file() and not copied_db.is_symlink():
            report["copied_database_final_sha256"] = digest(copied_db)
            report["copied_database_final_counts"] = database_counts(copied_db)
        report["output_bytes"] = folder_bytes(output)
        report["source_database_unchanged"] = stock.same_digest(source / "database.db", database["sha256"])
        report["source_images_unchanged"] = all(stock.same_digest(
            source / "images" / item["name"], item["sha256"]) for item in images)
        report["copied_images_unchanged"] = all(stock.same_digest(
            output / "images" / item["name"], item["sha256"]) for item in images)
        report["source_report_unchanged"] = stock.same_digest(
            source / "report.json", database["source_report_sha256"])
        report["worker_config_unchanged"] = (
            "worker_config_sha256" not in report or stock.same_digest(
                output / "worker-config.json", report["worker_config_sha256"]))
        report["free_bytes_after"] = {"output": shutil.disk_usage(output).free,
                                      "workspace": shutil.disk_usage(ROOT).free}
        try:
            current = stock.toolchain(python)
            current["fixed_intrinsics_runner_sha256"] = digest(Path(__file__))
            report["software_unchanged"] = current == software
        except Exception:
            report["software_unchanged"] = False
        report["partial_model_files_sha256"] = stock.partial_model_hashes(output)
        report["elapsed_seconds_total"] = round(stock.MAX_SECONDS - (deadline - time.monotonic()), 3)
        violations = []
        if not all(report[key] for key in ("source_database_unchanged", "source_images_unchanged",
                                           "copied_images_unchanged", "source_report_unchanged",
                                           "worker_config_unchanged", "software_unchanged")):
            violations.append("source, copy, or software hash changed")
        if report["output_bytes"] > stock.MAX_BYTES:
            violations.append("output byte limit exceeded")
        if any(free < RESERVE for free in report["free_bytes_after"].values()):
            violations.append("10 GiB disk reserve reached")
        if time.monotonic() > deadline:
            violations.append("total deadline exceeded")
        if violations:
            report.update(status="failed", failure="; ".join(violations),
                          structural_sparse_eligible_pending_camera_review=False)
        save()
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, default=SOURCE)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--arm", choices=ARMS, required=True)
    parser.add_argument("--python", type=Path, default=PYTHON)
    parser.add_argument("--worker", action="store_true")
    args = parser.parse_args()
    if args.worker:
        if json.loads((args.output / "worker-config.json").read_text())["arm"] != args.arm:
            raise ValueError("worker arm differs from sealed worker configuration")
        worker(args.output)
        return 0
    result = run(args.source, args.output, args.arm, args.python)
    print(json.dumps({"status": result["status"], "output": str(args.output),
                      "best_model_index": result.get("best_model_index"),
                      "third_view_registered": result.get("third_view_registered"),
                      "failure": result.get("failure")}, indent=2))
    return 0 if result["status"] == "complete" else 1


if __name__ == "__main__":
    raise SystemExit(main())
