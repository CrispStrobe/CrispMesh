"""Fail-closed OpenMVG HIGH rough-mesh → OpenMVS refine/texture continuation.

Default invocation is a read-only preflight. Native work requires an exact
rough receipt and a separately hash-bound positive native/visual review.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import time

from scripts.classical_backend.run import checked_ply, digest, folder_bytes, obj_counts
from scripts.classical_backend.openmvg_bunny_high_cache_rough import elapsed_ps_seconds
from scripts.classical_backend.openmvg_photo_control import base as supervisor


ROOT = Path(__file__).resolve().parents[2]
SOURCE = Path("/Volumes/backups/code/crispmesh-data/openmvg-bunny-high-openmvs-cache-fusion-003")
OUTPUT = Path("/Volumes/backups/code/crispmesh-data/openmvg-bunny-high-openmvs-refine-004")
BIN = ROOT / ".local-tools/classical-backend/bin"
SCHEMA = "openmvg_bunny_high_refine_texture_v3"
ROUGH_SCHEMA = "openmvg_bunny_high_cached73_fusion_rough_v1"
RESCUE_SCHEMA = "openmvg_bunny_high_rough_rescue_v1"
REVIEW_SCHEMA = "openmvg_bunny_high_rough_visual_review_v1"
NAMES = {f"frame_{i:04}.png" for i in range(73)}
TOOLS = ("RefineMesh", "TextureMesh")
CAP = 256 << 20
RESERVE = 11 << 30
RSS_CAP_KIB = 4 << 20
LOG_CAP = 16 << 20
DEADLINE_SECONDS = 20 * 60
MIN_NEW_OUTPUT_HEADROOM = 96 << 20
CAFFEINATE = Path("/usr/bin/caffeinate")


def _real_file(path: Path) -> Path:
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"missing or linked input: {path}")
    return path.resolve()


def _sha256_arg(value: str) -> str:
    if len(value) != 64 or any(char not in "0123456789abcdef" for char in value):
        raise argparse.ArgumentTypeError("expected lowercase SHA-256 hex")
    return value


def validate(args: argparse.Namespace) -> dict:
    source_receipt = _real_file(args.source / "result.json")
    rescue_path = _real_file(args.rescue_receipt)
    review_path = _real_file(args.review_receipt)
    source_sha = digest(source_receipt)
    rescue_sha = digest(rescue_path)
    review_sha = digest(review_path)
    if (source_sha != args.source_receipt_sha256 or
            rescue_sha != args.rescue_receipt_sha256 or
            review_sha != args.review_receipt_sha256):
        raise ValueError("rough, rescue, or native/visual review receipt hash differs")
    source = json.loads(source_receipt.read_text())
    rescue = json.loads(rescue_path.read_text())
    review = json.loads(review_path.read_text())
    if (source.get("schema") != ROUGH_SCHEMA or
            source.get("status") != "failed" or
            source.get("failure") != f"[Errno 2] No such file or directory: '{args.source / 'mesh.mvs'}'" or
            source.get("source_unchanged") is not True or
            source.get("quality_accepted") is not False):
        raise ValueError("rough source does not match sealed missing-mesh.mvs receipt")
    if (rescue.get("schema") != RESCUE_SCHEMA or
            rescue.get("status") != "native_rough_complete_pending_visual_review" or
            rescue.get("rough_source") != str(args.source.resolve()) or
            rescue.get("failed_receipt_sha256") != source_sha or
            rescue.get("native_mesh_valid") is not True or
            rescue.get("source_unchanged") is not True or
            rescue.get("cached_base_dmaps_verified") != 73 or
            rescue.get("converted_images_verified") != 73 or
            rescue.get("visual_object_shape_reviewed") is not False or
            rescue.get("quality_accepted") is not False or
            rescue.get("mesh_geometry", {}).get("zero_area_faces") != 0):
        raise ValueError("rescue sidecar does not seal this native rough run")
    stages = {row.get("name"): row for row in source.get("stages", []) if isinstance(row, dict)}
    if any(stages.get(name, {}).get("status") != "complete" for name in ("densify", "mesh")):
        raise ValueError("rough densify/mesh stage not sealed complete")
    densify = stages["densify"]
    command = densify.get("command", [])
    def has_option(flag: str, value: str) -> bool:
        return flag in command and command[command.index(flag) + 1:command.index(flag) + 2] == [value]
    if (densify.get("cache_fusion_verified") is not True or
            not isinstance(command, list) or
            not has_option("--geometric-iters", "0") or
            not has_option("--resolution-level", "3")):
        raise ValueError("rough fusion was not verified with frozen geom0 profile")
    inventory = source.get("output_inventory", {})
    artifacts = rescue.get("native_artifacts", {})
    if not isinstance(inventory, dict) or not isinstance(artifacts, dict):
        raise ValueError("rough receipt lacks artifact inventory")
    core = {}
    for name in ("dense.mvs", "dense.ply", "mesh.ply"):
        path = _real_file(args.source / name)
        expected = artifacts.get(name, {}).get("sha256")
        if (not isinstance(expected, str) or digest(path) != expected or
                inventory.get(name, {}).get("sha256") != expected or
                artifacts[name].get("path") != str(path) or
                artifacts[name].get("bytes") != path.stat().st_size):
            raise ValueError(f"rough native artifact differs: {name}")
        core[name] = expected
    for stage_name, artifact_name in (("densify", "dense.ply"), ("mesh", "mesh.ply")):
        stage_artifact = stages[stage_name].get("artifact", {})
        rescued_stage = rescue.get("stage_status", {}).get(stage_name, {})
        if (stage_artifact.get("sha256") != core[artifact_name] or
                stage_artifact.get("path") != str((args.source / artifact_name).resolve()) or
                stage_artifact.get("bytes") != (args.source / artifact_name).stat().st_size or
                stages[stage_name].get("returncode") != 0 or
                rescued_stage.get("status") != "complete" or
                rescued_stage.get("returncode") != 0 or
                rescued_stage.get("log_sha256") != stages[stage_name].get("log_sha256") or
                rescued_stage.get("artifact_sha256") != core[artifact_name]):
            raise ValueError(f"rough {stage_name} stage does not seal its native artifact")
    rough_faces = checked_ply(args.source / "mesh.ply", "face")
    dense_points = checked_ply(args.source / "dense.ply", "vertex")
    if rough_faces < 1 or dense_points < 1:
        raise ValueError("rough geometry is empty")
    images = args.source / "images"
    if (images.is_symlink() or not images.is_dir() or
            {path.name for path in images.iterdir()} != NAMES):
        raise ValueError("rough source image inventory differs from 73 processed photos")
    image_sha = {}
    for name in sorted(NAMES):
        path = _real_file(images / name)
        expected = inventory.get(f"images/{name}", {}).get("sha256")
        if not isinstance(expected, str) or digest(path) != expected:
            raise ValueError(f"rough source image differs: {name}")
        image_sha[name] = expected
    if (review.get("schema") != REVIEW_SCHEMA or
            review.get("decision") != "approved_for_refine_texture" or
            review.get("native_mesh_valid") is not True or
            review.get("visual_object_shape_reviewed") is not True or
            not isinstance(review.get("reviewer"), str) or not review["reviewer"].strip() or
            review.get("rescue_receipt_sha256") != rescue_sha or
            review.get("dense_mvs_sha256") != core["dense.mvs"] or
            review.get("mesh_ply_sha256") != core["mesh.ply"]):
        raise ValueError("separate native/visual review does not approve exact rough mesh")
    binaries = {}
    for name in TOOLS:
        path = _real_file(args.binary_dir / name)
        if not os.access(path, os.X_OK):
            raise ValueError(f"native tool not executable: {name}")
        binaries[name] = {"path": str(path), "sha256": digest(path)}
    if not CAFFEINATE.is_file() or not os.access(CAFFEINATE, os.X_OK):
        raise ValueError("macOS caffeinate is unavailable")
    if (args.output.exists() or args.output.is_symlink() or
            args.output.parent.is_symlink() or not args.output.parent.is_dir()):
        raise ValueError("continuation output must be a fresh path")
    copies = sum((images / name).stat().st_size for name in NAMES)
    copies += (args.source / "dense.mvs").stat().st_size + (args.source / "mesh.ply").stat().st_size
    if copies + MIN_NEW_OUTPUT_HEADROOM > CAP:
        raise ValueError("source copies leave less than 96 MiB for refined and textured outputs")
    for volume in {args.output.parent.resolve(), args.source.resolve(), ROOT}:
        if shutil.disk_usage(volume).free < RESERVE + CAP:
            raise ValueError(f"11 GiB reserve plus 256 MiB headroom unavailable: {volume}")
    return {"source": str(args.source.resolve()), "source_receipt_sha256": source_sha,
            "rescue_receipt": str(rescue_path), "rescue_receipt_sha256": rescue_sha,
            "review_receipt": str(review_path), "review_receipt_sha256": review_sha,
            "rough_artifact_sha256": core, "image_sha256": image_sha,
            "rough_faces": rough_faces, "dense_points": dense_points,
            "binary": binaries, "copy_bytes": copies}


def commands(output: Path, binding: dict) -> list[tuple[str, list[str], str]]:
    common = ["--max-threads", "2", "--working-folder", str(output)]
    return [
        ("refine", [binding["binary"]["RefineMesh"]["path"], "-i", str(output / "dense.mvs"),
                    "-m", str(output / "mesh.ply"), "-o", str(output / "refined.mvs"),
                    "--resolution-level", "2", "--scales", "1", *common], "refined.ply"),
        ("texture", [binding["binary"]["TextureMesh"]["path"], "-i", str(output / "dense.mvs"),
                     "-m", str(output / "refined.ply"), "-o", str(output / "textured.mvs"),
                     "--export-type", "obj", *common], "textured.obj"),
    ]


def _kill(process: subprocess.Popen) -> None:
    if process.poll() is None:
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait()


def _stage(output: Path, name: str, command: list[str], started_epoch: float,
           reserve_paths: tuple[Path, ...]) -> dict:
    log_path = output / f"{name}.log"
    env = os.environ.copy()
    env.update(OPENBLAS_NUM_THREADS="2", OMP_NUM_THREADS="2", TMPDIR=str(output / "tmp"))
    started = time.time()
    reason = None
    peak_kib = 0
    with log_path.open("wb") as log:
        process = subprocess.Popen([str(CAFFEINATE), "-disu", *command], cwd=output, env=env, stdout=log,
                                   stderr=subprocess.STDOUT, start_new_session=True)
        try:
            while process.poll() is None:
                ps_elapsed = elapsed_ps_seconds(process.pid)
                if (time.time() - started_epoch >= DEADLINE_SECONDS or
                        (ps_elapsed is not None and ps_elapsed >= DEADLINE_SECONDS)):
                    reason = "20-minute whole-continuation deadline reached"
                elif (folder_bytes(output) > CAP or log_path.stat().st_size > LOG_CAP or
                      any(shutil.disk_usage(path).free < RESERVE for path in reserve_paths)):
                    reason = "output, log, or 11 GiB disk cap reached"
                else:
                    table = subprocess.check_output(["/bin/ps", "-axo", "pid=,ppid=,rss="],
                                                    text=True)
                    peak_kib = max(peak_kib, supervisor.process_rss_kib(process.pid, table))
                    if peak_kib > RSS_CAP_KIB:
                        reason = "4 GiB child RSS cap reached"
                if reason:
                    break
                time.sleep(0.5)
        finally:
            if process.poll() is None:
                _kill(process)
    code = process.wait()
    if code != 0:
        reason = reason or f"exit status {code}"
    if folder_bytes(output) > CAP or any(shutil.disk_usage(path).free < RESERVE for path in reserve_paths):
        reason = reason or "resource cap exceeded after stage"
    return {"name": name, "command": command, "returncode": code,
            "status": "complete" if reason is None else "failed", "failure": reason,
            "real_wall_seconds": time.time() - started,
            "peak_process_tree_rss_kib": peak_kib, "log": str(log_path)}


def execute(args: argparse.Namespace) -> dict:
    bound = validate(args)  # No output until exact rough and visual gates pass.
    started_epoch = time.time()  # Host wall clock includes sleep and input copy.
    output = args.output.resolve()
    output.mkdir()
    (output / "tmp").mkdir()
    reserve_paths = tuple({output, ROOT, Path(bound["source"])})
    report = {"schema": SCHEMA, "status": "running", "source": bound,
              "composition": "OpenMVG HIGH sparse + cached-depth rough + separately gated refine/texture",
              "limits": {"max_output_bytes": CAP, "reserve_bytes": RESERVE,
                         "max_threads": 2, "global_deadline_seconds": DEADLINE_SECONDS,
                         "minimum_new_output_headroom_bytes": MIN_NEW_OUTPUT_HEADROOM,
                         "max_child_rss_kib": RSS_CAP_KIB, "max_stage_log_bytes": LOG_CAP},
              "stages": [], "quality_accepted": False, "metric_scale_verified": False,
              "shipping_approved": False}

    def save() -> None:
        (output / "result.json").write_text(json.dumps(report, indent=2) + "\n")

    save()
    try:
        (output / "images").mkdir()
        for name, expected in sorted(bound["image_sha256"].items()):
            if time.time() - started_epoch >= DEADLINE_SECONDS:
                raise TimeoutError("20-minute deadline reached during input copy")
            source, target = Path(bound["source"]) / "images" / name, output / "images" / name
            if source.is_symlink() or digest(source) != expected:
                raise ValueError(f"source image changed before copy: {name}")
            shutil.copyfile(source, target)
            if digest(target) != expected:
                raise ValueError(f"copied image differs: {name}")
            if folder_bytes(output) > CAP or any(shutil.disk_usage(p).free < RESERVE for p in reserve_paths):
                raise ValueError("output or disk cap reached while copying images")
        for name in ("dense.mvs", "mesh.ply"):
            if time.time() - started_epoch >= DEADLINE_SECONDS:
                raise TimeoutError("20-minute deadline reached during native input copy")
            source, target = Path(bound["source"]) / name, output / name
            if source.is_symlink() or digest(source) != bound["rough_artifact_sha256"][name]:
                raise ValueError(f"rough artifact changed before copy: {name}")
            shutil.copyfile(source, target)
            if digest(target) != bound["rough_artifact_sha256"][name]:
                raise ValueError(f"copied rough artifact differs: {name}")
            if folder_bytes(output) > CAP or any(shutil.disk_usage(p).free < RESERVE for p in reserve_paths):
                raise ValueError("output or disk cap reached while copying native inputs")
        report["stages"].append({"name": "copy_inputs", "status": "complete",
                                 "copied_bytes": bound["copy_bytes"]})
        save()
        for name, command, artifact_name in commands(output, bound):
            if time.time() - started_epoch >= DEADLINE_SECONDS:
                raise TimeoutError("20-minute deadline reached before native stage")
            if any(digest(Path(info["path"])) != info["sha256"]
                   for info in bound["binary"].values()):
                raise ValueError("native binary changed before stage")
            result = _stage(output, name, command, started_epoch, reserve_paths)
            report["stages"].append(result)
            save()
            if result["status"] != "complete":
                raise RuntimeError(f"{name}: {result['failure']}")
            artifact = output / artifact_name
            if artifact.is_symlink() or not artifact.is_file():
                raise ValueError(f"native artifact missing or linked: {artifact_name}")
            if name == "refine":
                result["faces"] = checked_ply(artifact, "face")
            else:
                result["vertices"], result["faces"] = obj_counts(artifact)
            result["artifact_sha256"] = digest(artifact)
            save()
        if time.time() - started_epoch >= DEADLINE_SECONDS:
            raise TimeoutError("20-minute deadline reached before finalization")
        report["status"] = "complete_conditional_continuation_only"
    except Exception as error:
        report.update(status="failed", failure=str(error))
    report["output_bytes"] = folder_bytes(output)
    report["real_wall_seconds_total"] = time.time() - started_epoch
    report["free_bytes_after"] = {str(path): shutil.disk_usage(path).free for path in reserve_paths}
    try:
        report["source_unchanged"] = (digest(Path(bound["source"]) / "result.json") == bound["source_receipt_sha256"] and
            digest(Path(bound["rescue_receipt"])) == bound["rescue_receipt_sha256"] and
            digest(Path(bound["review_receipt"])) == bound["review_receipt_sha256"] and
            all(digest(Path(bound["source"]) / "images" / name) == sha
                for name, sha in bound["image_sha256"].items()) and
            all(digest(Path(bound["source"]) / name) == sha
                for name, sha in bound["rough_artifact_sha256"].items()) and
            all(digest(Path(info["path"])) == info["sha256"] for info in bound["binary"].values()))
    except Exception:
        report["source_unchanged"] = False
    if (not report["source_unchanged"] or report["output_bytes"] > CAP or
            report["real_wall_seconds_total"] > DEADLINE_SECONDS or
            any(value < RESERVE for value in report["free_bytes_after"].values())):
        report.update(status="failed", failure="source or resource postcheck failed")
    save()
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, default=SOURCE)
    parser.add_argument("--source-receipt-sha256", type=_sha256_arg, required=True)
    parser.add_argument("--rescue-receipt", type=Path, required=True)
    parser.add_argument("--rescue-receipt-sha256", type=_sha256_arg, required=True)
    parser.add_argument("--review-receipt", type=Path, required=True)
    parser.add_argument("--review-receipt-sha256", type=_sha256_arg, required=True)
    parser.add_argument("--output", type=Path, default=OUTPUT)
    parser.add_argument("--binary-dir", type=Path, default=BIN)
    parser.add_argument("--execute", action="store_true")
    args = parser.parse_args()
    try:
        if args.execute:
            report = execute(args)
        else:
            bound = validate(args)
            report = {"schema": SCHEMA, "status": "read_only_preflight",
                      "source": bound, "output": str(args.output),
                      "commands": [row[1] for row in commands(args.output, bound)]}
    except Exception as error:
        print(json.dumps({"status": "abstained", "reason": str(error)}))
        return 2
    print(json.dumps({"status": report["status"], "result": str(args.output / "result.json"),
                      "failure": report.get("failure")}))
    return 0 if report["status"] != "failed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
