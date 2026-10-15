#!/usr/bin/env python3
"""Fresh, bounded replay of the proven image-only turntable sparse profile.

This intentionally invokes the existing PyCOLMAP foreground worker unchanged.
It does not use Berkeley poses, calibration, depth, or reference geometry.
"""

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import threading
import time

from scripts.object_motion import run


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_EXTERNAL_ROOT = Path("/Volumes/backups/code/crispmesh-data")
OUTPUT_CAP = 512 * 1024**2
FREE_FLOOR = 10 * 1024**3
LOG_CAP = 4 * 1024**2
RSS_CAP = 4 * 1024**3
POLL_SECONDS = 1


def digest(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def input_seal(manifest_path):
    """Revalidate every original photo and mask, then bind the input file bytes."""
    manifest_path = Path(manifest_path)
    if manifest_path.is_symlink() or not manifest_path.is_file():
        raise ValueError("manifest must be a regular file")
    manifest = json.loads(manifest_path.read_text())
    images = run.validate(manifest)
    return {"manifest": digest(manifest_path),
            "source_manifest": manifest["source_manifest_sha256"],
            "photos": {r["name"]: r["sha256"] for r in images},
            "masks": {r["name"]: r["mask_sha256"] for r in images}}


def is_external_filesystem(root):
    return os.stat(root).st_dev != os.stat(ROOT).st_dev


def validate_output(output, external_root):
    """Require a new directory on the selected external filesystem."""
    if Path(external_root).is_symlink():
        raise ValueError("external root must not be a symlink")
    root = Path(external_root).resolve(strict=True)
    if not root.is_dir():
        raise ValueError("external root must be a real directory")
    if not is_external_filesystem(root):
        raise ValueError("external root must be on a different filesystem")
    output = Path(output)
    if output.exists() or output.is_symlink():
        raise FileExistsError(output)
    parent = output.parent.resolve(strict=True)
    if not parent.is_dir() or not parent.is_relative_to(root):
        raise ValueError("output parent must be inside external root")
    if shutil.disk_usage(parent).free < FREE_FLOOR + OUTPUT_CAP:
        raise RuntimeError("external output lacks 10 GiB reserve plus 512 MiB budget")
    if shutil.disk_usage(ROOT).free < FREE_FLOOR:
        raise RuntimeError("less than 10 GiB free on internal workspace")
    return parent / output.name


def output_bytes(path):
    size = 0
    for p in path.rglob("*"):
        try:
            if p.is_file():
                size += p.stat().st_size
        except FileNotFoundError:  # native worker atomically replaces temporary files
            continue
    return size


def verify_model_hashes(summary, destination):
    expected = summary.get("model_files_sha256") or {}
    model_dir = Path(summary.get("model_dir", ""))
    names = {"cameras.bin", "images.bin", "points3D.bin"}
    return (set(expected) == names and model_dir.is_dir() and
            model_dir.resolve().is_relative_to(destination.resolve()) and
            all((model_dir / name).is_file() and not (model_dir / name).is_symlink() and
                digest(model_dir / name) == expected[name] for name in names))


def fixed_mapping_options(pycolmap):
    """Use the image-derived database camera heuristic until the full orbit exists."""
    options = pycolmap.IncrementalPipelineOptions()
    options.num_threads = 2
    options.mapper.num_threads = 2
    options.multiple_models = False
    options.max_num_models = 1
    options.min_model_size = 2
    options.ba_refine_focal_length = False
    options.ba_refine_principal_point = False
    options.ba_refine_extra_params = False
    options.mapper.abs_pose_refine_focal_length = False
    options.mapper.abs_pose_refine_extra_params = False
    return options


def delayed_ba_options(pycolmap):
    options = pycolmap.BundleAdjustmentOptions()
    options.refine_focal_length = True
    options.refine_principal_point = False
    options.refine_extra_params = True
    options.refine_extrinsics = True
    options.use_gpu = False
    options.print_summary = True
    options.solver_options.max_num_iterations = 50
    options.solver_options.max_solver_time_in_seconds = 50.0
    options.solver_options.num_threads = 2
    return options


def extract_and_match_fresh(pycolmap, images, database):
    """Recreate the original masked foreground feature and match configuration."""
    image_dir = Path(images[0]["path"]).parent
    reader = pycolmap.ImageReaderOptions()
    reader.camera_model = "SIMPLE_RADIAL"
    reader.default_focal_length_factor = 1.2
    reader.mask_path = str(Path(images[0]["pose_support_mask"]).parent)
    sift = pycolmap.SiftExtractionOptions()
    sift.max_image_size = 1200
    sift.max_num_features = 1800
    sift.num_threads = 2
    matching = pycolmap.SiftMatchingOptions()
    matching.num_threads = 2
    sequential = pycolmap.SequentialMatchingOptions()
    sequential.overlap = 8
    sequential.quadratic_overlap = False
    sequential.loop_detection = False
    verification = pycolmap.TwoViewGeometryOptions()
    pycolmap.extract_features(str(database), str(image_dir), [r["name"] for r in images],
                              camera_mode=pycolmap.CameraMode.SINGLE,
                              camera_model=reader.camera_model, reader_options=reader,
                              sift_options=sift, device=pycolmap.Device.cpu)
    pycolmap.match_sequential(str(database), sift_options=matching,
                              matching_options=sequential,
                              verification_options=verification, device=pycolmap.Device.cpu)
    return {"camera_mode": "SINGLE", "image_reader": reader.todict(),
            "sift": sift.todict(), "matching": matching.todict(),
            "sequential": sequential.todict(), "verification": verification.todict()}


def fixed_cache_worker(manifest_path, source_database, output):
    """Map fresh features, optionally from a sealed failed automatic replay cache."""
    import pycolmap
    from scripts.object_motion.diagnose import diagnose
    from scripts.object_motion.initialization_recovery import read_cache_logical
    manifest_path = Path(manifest_path)
    output = Path(output)
    manifest = json.loads(manifest_path.read_text())
    images = run.validate(manifest)
    database = output / "database.db"
    options = fixed_mapping_options(pycolmap)
    pycolmap.set_random_seed(20260927)
    if source_database is None:
        extraction = extract_and_match_fresh(pycolmap, images, database)
    else:
        source_database = Path(source_database)
        source_wal = Path(str(source_database) + "-wal")
        if source_wal.exists() and source_wal.stat().st_size:
            raise ValueError("source database has nonempty WAL")
        shutil.copyfile(source_database, database)
        if digest(database) != digest(source_database):
            raise ValueError("fresh database copy differs")
        extraction = None
    source_hash = digest(database)
    source_logical = read_cache_logical(database)
    provenance = {"schema": "turntable_fixed_fresh_cache_worker_v1",
                  "input_manifest_sha256": digest(manifest_path),
                  "source_database_sha256": source_hash,
                  "source_database_logical_tables": source_logical["tables"],
                  "source_database": str(source_database) if source_database else None,
                  "fresh_extraction_and_matching": extraction,
                  "photo_hashes": {r["name"]: r["sha256"] for r in images},
                  "mask_hashes": {r["name"]: r["mask_sha256"] for r in images},
                  "pycolmap_version": pycolmap.__version__,
                  "pycolmap_binary_sha256": digest(pycolmap._core.__file__),
                  "mapping_options": options.todict(),
                  "seed_policy": "PyCOLMAP automatic initial pair; no supplied poses",
                  "intrinsics_origin": "fresh ImageReader SIMPLE_RADIAL default focal factor 1.2; no supplied calibration"}
    (output / "provenance.json").write_text(json.dumps(provenance, indent=2, default=str) + "\n")
    started = time.monotonic()
    models = pycolmap.incremental_mapping(str(database), str(Path(images[0]["path"]).parent),
                                          str(output / "models_fixed"), options=options)
    fixed = max(models.values(), key=lambda model: (model.num_reg_images(), model.num_points3D())) if models else None
    fixed_count = fixed.num_reg_images() if fixed else 0
    if fixed is None or fixed_count != len(images):
        summary = {"model_count": len(models), "registered": fixed_count,
                   "points3D": fixed.num_points3D() if fixed else 0,
                   "status": "fixed_mapping_incomplete", "fixed_mapping_seconds": time.monotonic() - started}
        (output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
        return 1
    camera = next(iter(fixed.cameras.values())) if len(fixed.cameras) == 1 else None
    if (camera is None or camera.model.name != "SIMPLE_RADIAL" or
            list(map(float, camera.params)) != [1536.0, 640.0, 512.0, 0.0]):
        raise ValueError("fixed mapping did not retain image-only initial camera heuristic")
    ba_options = delayed_ba_options(pycolmap)
    pycolmap.bundle_adjustment(fixed, ba_options)
    refined_camera = next(iter(fixed.cameras.values())) if len(fixed.cameras) == 1 else None
    params = list(map(float, refined_camera.params)) if refined_camera else []
    plausible = (len(params) == 4 and all(math.isfinite(v) for v in params)
                 and 0.3 <= params[0] / 1280 <= 3 and abs(params[3]) <= 0.5
                 and params[1:3] == [640.0, 512.0])
    model_dir = output / "models" / "0"
    model_dir.mkdir(parents=True)
    fixed.write(str(model_dir))
    summary = {"model_count": 1, "registered": fixed.num_reg_images(),
               "points3D": fixed.num_points3D(), "status": "complete" if plausible else "intrinsics_gate_failed",
               "fixed_mapping_registered": fixed_count, "final_camera_params": params,
               "fixed_mapping_and_ba_seconds": time.monotonic() - started,
               "delayed_ba_options": ba_options.todict()}
    summary.update(diagnose(fixed, manifest))
    run.validate(manifest)
    copied_logical = read_cache_logical(database)
    if ((source_database is not None and digest(source_database) != source_hash) or
            copied_logical["tables"] != source_logical["tables"] or
            copied_logical["schemas_sha256"] != source_logical["schemas_sha256"]):
        raise ValueError("source bytes or copied fresh database logical tables changed during mapping")
    summary["source_database_sha256"] = source_hash
    summary["copied_database_sha256_after"] = digest(database)
    summary["copied_database_logical_tables_unchanged"] = True
    (output / "summary.json").write_text(json.dumps(summary, indent=2, default=str) + "\n")
    run.seal(output)
    return 0 if plausible else 1


def _drain(pipe, log_path, state):
    used = 0
    with log_path.open("wb") as log:
        for chunk in iter(lambda: pipe.read(65536), b""):
            room = max(0, LOG_CAP - used)
            if room:
                part = chunk[:room]
                log.write(part)
                used += len(part)
            if len(chunk) > room:
                state["log_truncated"] = True
        if state["log_truncated"]:
            log.write(b"\n[log capped at 4 MiB; child output fully drained]\n")


def supervise(command, output, timeout_seconds):
    """Bound the fresh worker by wall time, output bytes, and remaining space."""
    output.mkdir()
    temp = output / "tmp"
    temp.mkdir()
    env = os.environ.copy()
    env.update(TMPDIR=str(temp), TMP=str(temp), OPENBLAS_NUM_THREADS="2", OMP_NUM_THREADS="2",
               PYTHONPATH=str(ROOT) + os.pathsep + env.get("PYTHONPATH", ""))
    state = {"log_truncated": False}
    start = time.monotonic()
    process = subprocess.Popen(command, cwd=output, env=env,
                               stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                               start_new_session=True)
    drain = threading.Thread(target=_drain, args=(process.stdout, output / "worker.log", state), daemon=True)
    drain.start()
    reason = None
    peak_rss = 0
    try:
        while process.poll() is None:
            if time.monotonic() - start >= timeout_seconds:
                reason = "timeout"
            elif output_bytes(output) > OUTPUT_CAP:
                reason = "output_cap"
            elif shutil.disk_usage(output).free < FREE_FLOOR:
                reason = "external_disk_floor"
            elif shutil.disk_usage(ROOT).free < FREE_FLOOR:
                reason = "internal_disk_floor"
            if process.poll() is None:
                sample = subprocess.run(["ps", "-o", "rss=", "-p", str(process.pid)],
                                        capture_output=True, text=True, check=False)
                if sample.returncode == 0 and sample.stdout.strip().isdigit():
                    peak_rss = max(peak_rss, int(sample.stdout.strip()) * 1024)
                    if peak_rss > RSS_CAP:
                        reason = "rss_cap"
            if reason:
                os.killpg(process.pid, signal.SIGKILL)
                break
            time.sleep(POLL_SECONDS)
        code = process.wait()
    except BaseException:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGKILL)
        process.wait()
        raise
    finally:
        drain.join(timeout=10)
        process.stdout.close()
    if reason is None:
        if output_bytes(output) > OUTPUT_CAP:
            reason = "output_cap"
        elif shutil.disk_usage(output).free < FREE_FLOOR:
            reason = "external_disk_floor"
        elif shutil.disk_usage(ROOT).free < FREE_FLOOR:
            reason = "internal_disk_floor"
    return {"status": reason or ("completed" if code == 0 else "worker_failed"),
            "returncode": code, "wall_seconds": time.monotonic() - start,
            "timeout_seconds": timeout_seconds, "output_bytes": output_bytes(output),
            "peak_sampled_child_rss_bytes": peak_rss, **state}


def run_profile(manifest_path, output, external_root, timeout_seconds,
                source_database=None, fresh_fixed=False):
    if not 1 <= timeout_seconds <= 3600:
        raise ValueError("timeout must be 1..3600 seconds")
    before = input_seal(manifest_path)
    manifest_path = Path(manifest_path).resolve(strict=True)
    destination = validate_output(output, external_root)
    if source_database is not None and fresh_fixed:
        raise ValueError("fresh fixed mode cannot also use a cached source database")
    if fresh_fixed:
        command = [str(run.VENV_PYTHON), str(Path(__file__)), "--fixed-worker", "--fresh-fixed",
                   "--manifest", str(manifest_path), "--output", str(destination)]
    elif source_database is None:
        command = [str(run.VENV_PYTHON), str(ROOT / "scripts/object_motion/run.py"),
                   "--worker", "--arm", "foreground", "--manifest", str(manifest_path),
                   "--output", str(destination)]
    else:
        if Path(source_database).is_symlink():
            raise ValueError("source database must not be a symlink")
        source_database = Path(source_database).resolve(strict=True)
        if source_database.name != "database.db" or not source_database.is_file():
            raise ValueError("source must be a regular fresh PyCOLMAP database")
        source_provenance = json.loads((source_database.parent / "provenance.json").read_text())
        if (source_provenance.get("input_manifest_sha256") != before["manifest"] or
                source_provenance.get("image_hashes") != before["photos"] or
                source_provenance.get("mask_hashes") != before["masks"]):
            raise ValueError("fresh cache provenance does not match sealed inputs")
        command = [str(run.VENV_PYTHON), str(Path(__file__)), "--fixed-worker",
                   "--manifest", str(manifest_path), "--source-database", str(source_database),
                   "--output", str(destination)]
    profile_name = ("fresh_masked_fixed_intrinsics_delayed_ba" if fresh_fixed else
                    "prior_foreground_60_jpeg_replay" if source_database is None else
                    "fixed_intrinsics_delayed_ba_from_fresh_cache_trial")
    profile = {"schema": "turntable_sparse_profile_v1", "profile": profile_name,
               "input_seal": before, "runner_sha256": digest(__file__),
               "worker_sha256": digest(ROOT / "scripts/object_motion/run.py"),
               "diagnose_sha256": digest(ROOT / "scripts/object_motion/diagnose.py"),
               "command": command, "output_cap_bytes": OUTPUT_CAP, "free_floor_bytes": FREE_FLOOR,
               "child_rss_cap_bytes": RSS_CAP,
               "source_database_sha256": digest(source_database) if source_database else None,
               "timeout_seconds": timeout_seconds,
               "implemented": ["coarse photo-derived feature masks", "single shared SIMPLE_RADIAL camera",
                               "ordered sequential matching with overlap 8", "one incremental model"] +
                              (["fixed initial intrinsic parameters during mapping",
                                "delayed global bundle adjustment of focal/distortion/extrinsics"]
                               if fresh_fixed or source_database else []),
               "unimplemented_upstream_guidance": ["Meshroom FeatureMatching Minimal 2D Motion 2px",
                                                    "dense-stage masks"],
               "qa_scope": "image-only sparse diagnostics; no independent pose truth, scale, or mesh acceptance",
               "independent_camera_gate_passed": None}
    status = supervise(command, destination, timeout_seconds)
    after = input_seal(manifest_path)
    status["inputs_unchanged"] = before == after
    if not status["inputs_unchanged"]:
        status["status"] = "input_changed"
    summary = destination / "summary.json"
    if status["status"] == "completed":
        if not summary.is_file():
            status["status"] = "missing_summary"
        else:
            result = json.loads(summary.read_text())
            status["registered"] = result.get("registered", 0)
            status["points3D"] = result.get("points3D", 0)
            status["sparse_coverage_passed"] = result.get("registered") == len(before["photos"])
            status["summary_sha256"] = digest(summary)
            status["model_files_sha256"] = result.get("model_files_sha256")
            status["model_hashes_verified"] = verify_model_hashes(result, destination)
            if not status["sparse_coverage_passed"] or not status["model_hashes_verified"]:
                status["status"] = "sparse_qa_failed"
    profile["status"] = status
    (destination / "turntable-profile.json").write_text(json.dumps(profile, indent=2) + "\n")
    return profile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=run.DEFAULT_MANIFEST)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--external-root", type=Path, default=DEFAULT_EXTERNAL_ROOT)
    parser.add_argument("--timeout-seconds", type=int, default=600)
    parser.add_argument("--source-database", type=Path,
                        help="fresh image-derived feature cache for fixed-K mapping trial")
    parser.add_argument("--fresh-fixed", action="store_true",
                        help="fresh images/masks to masked features, matches, fixed-K SfM, then BA")
    parser.add_argument("--fixed-worker", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--check-only", action="store_true", help="validate inputs and destination without writing")
    args = parser.parse_args()
    if args.fixed_worker:
        if not args.fresh_fixed and args.source_database is None:
            parser.error("--fixed-worker requires --fresh-fixed or --source-database")
        raise SystemExit(fixed_cache_worker(args.manifest, args.source_database, args.output))
    if args.check_only:
        seal = input_seal(args.manifest)
        output = validate_output(args.output, args.external_root)
        print(json.dumps({"ready": True, "output": str(output), "images": len(seal["photos"]),
                          "input_manifest_sha256": seal["manifest"]}))
        return
    report = run_profile(args.manifest, args.output, args.external_root,
                         args.timeout_seconds, args.source_database, args.fresh_fixed)
    print(json.dumps(report["status"]))
    if report["status"]["status"] != "completed":
        raise SystemExit(1)


if __name__ == "__main__":
    main()
