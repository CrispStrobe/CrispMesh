#!/usr/bin/env python3
"""Compare SGBM, external ELAS, and project-authored Census on identical prepared inputs.

Example:
  python3 scripts/run_stereo_benchmarks.py \
    --prepared piano=.local-tools/oracles/prepared/piano \
    --elas .local-tools/oracles/elas/build/elas_oracle \
    --census .local-tools/oracles/census/census

The separate engines see only prepared left/right PGM and an exclusive disparity
count. Only the evaluator receives truth.pfm. This script does not tune engines.
"""

import argparse
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess
import sys
import time
import uuid


ROOT = Path(__file__).resolve().parent.parent
DEFAULT_EVALUATOR = ROOT / "build-opencv/bin/crispmesh_stereo_eval"
DEFAULT_OUTPUT = ROOT / "build-opencv/benchmarks"
RESERVE_BYTES = 10 * 1024**3


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def source_pins() -> dict[str, set[str]]:
    pins: dict[str, set[str]] = {}
    for manifest in sorted((ROOT / "tests/datasets").glob("*.json")):
        data = json.loads(manifest.read_text())
        for item in [*data.get("files", []), *data.get("derived", [])]:
            if isinstance(item, dict) and isinstance(item.get("path"), str) and re.fullmatch(r"[0-9a-f]{64}", str(item.get("sha256", ""))):
                pins.setdefault(item["path"], set()).add(item["sha256"])
    return pins


def pin_for(path: Path, pins: dict[str, set[str]]) -> tuple[str | None, str]:
    normalized = path.as_posix()
    matches = [(name, expected) for name, expected in pins.items() if normalized == name or normalized.endswith("/" + name)]
    if len(matches) > 1:
        raise ValueError(f"Ambiguous dataset pin for {path}")
    actual = digest(path)
    if not matches:
        return None, actual
    name, expected = matches[0]
    if actual not in expected:
        raise ValueError(f"Pinned source hash differs: {path} ({name})")
    return name, actual


def safe_prepared_file(folder: Path, value: object) -> Path:
    if not isinstance(value, str) or not value or Path(value).name != value:
        raise ValueError(f"Unsafe prepared filename: {value!r}")
    path = folder / value
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"Missing or symlinked prepared file: {path}")
    return path


def read_prepared(folder: Path, pins: dict[str, set[str]], allow_unpinned: bool) -> dict:
    folder = folder.resolve()
    manifest_path = safe_prepared_file(folder, "inputs.json")
    manifest = json.loads(manifest_path.read_text())
    if manifest.get("schema_version") != 1:
        raise ValueError(f"Unsupported prepared manifest: {manifest_path}")
    ndisp = manifest.get("ndisp", (manifest.get("calibration") or {}).get("ndisp"))
    if type(ndisp) is not int or not 1 <= ndisp <= 2048:
        raise ValueError(f"Invalid prepared disparity count: {ndisp}")
    width, height = manifest.get("width"), manifest.get("height")
    if type(width) is not int or type(height) is not int or not 8 <= width <= 10000 or not 8 <= height <= 10000:
        raise ValueError("Invalid prepared image dimensions")
    if ((ndisp + 15) // 16) * 16 >= width:
        raise ValueError("Common rounded disparity count must be smaller than image width")
    images = {key: safe_prepared_file(folder, manifest.get(key)) for key in ("left", "right", "truth")}
    optional_mask = manifest.get("mask")
    if optional_mask:
        images["mask"] = safe_prepared_file(folder, optional_mask)
    source_records = {}
    if not allow_unpinned and any(not isinstance(manifest.get(key), str) for key in ("source_left", "source_right", "source_gt")):
        raise ValueError("Prepared manifest needs source_left, source_right and source_gt")
    for key, value in manifest.items():
        if not key.startswith("source_") or not isinstance(value, str):
            continue
        source = Path(value)
        if not source.is_absolute():
            source = ROOT / source
        source = source.resolve()
        if not source.is_file():
            raise ValueError(f"Missing source file: {source}")
        pin, sha = pin_for(source, pins)
        if pin is None and not allow_unpinned:
            raise ValueError(f"No dataset SHA256 pin for source: {source}")
        source_records[key] = {"path": str(source), "sha256": sha, "dataset_pin": pin}
    if not source_records and not allow_unpinned:
        raise ValueError(f"Prepared manifest has no source provenance: {manifest_path}")
    calibration = manifest.get("calibration")
    if calibration is not None:
        if not isinstance(calibration, dict) or any(not isinstance(calibration.get(key), (int, float)) or not math.isfinite(calibration[key]) for key in ("fx", "baseline", "doffs", "ndisp")):
            raise ValueError("Invalid prepared calibration")
        if calibration["fx"] <= 0 or calibration["baseline"] <= 0 or calibration["ndisp"] != ndisp:
            raise ValueError("Invalid prepared calibration values")
    return {"folder": folder, "manifest": manifest, "manifest_sha256": digest(manifest_path),
            "ndisp": ndisp, "common_search_count": ((ndisp + 15) // 16) * 16,
            "files": images, "prepared_sha256": {key: digest(path) for key, path in images.items()},
            "sources": source_records}


def check_unchanged(prepared: dict) -> None:
    if digest(prepared["folder"] / "inputs.json") != prepared["manifest_sha256"]:
        raise ValueError("Prepared manifest changed during benchmark")
    for key, path in prepared["files"].items():
        if digest(path) != prepared["prepared_sha256"][key]:
            raise ValueError(f"Prepared {key} changed during benchmark")
    for record in prepared["sources"].values():
        if digest(Path(record["path"])) != record["sha256"]:
            raise ValueError("Source file changed during benchmark")


def execute(command: list[str], log: Path, timeout: int) -> dict:
    start = time.monotonic()
    try:
        process = subprocess.run(command, capture_output=True, text=True, timeout=timeout, check=False,
                                 env={**os.environ, "TMPDIR": str(ROOT / ".local-tools/tmp")})
        result = {"status": "ok" if process.returncode == 0 else "failed", "exit_code": process.returncode,
                  "elapsed_seconds": round(time.monotonic() - start, 3)}
        body = process.stdout + "\n--- stderr ---\n" + process.stderr
    except subprocess.TimeoutExpired as error:
        result = {"status": "timeout", "exit_code": None, "elapsed_seconds": round(time.monotonic() - start, 3)}
        stdout = error.stdout.decode(errors="replace") if isinstance(error.stdout, bytes) else error.stdout or ""
        stderr = error.stderr.decode(errors="replace") if isinstance(error.stderr, bytes) else error.stderr or ""
        body = stdout + "\n--- stderr ---\n" + stderr
    log.write_text(body[:100_000])
    return {**result, "command": command, "log": str(log)}


def validate_prediction(path: Path, width: int, height: int, disparities: int) -> dict:
    if path.is_symlink() or not path.is_file():
        raise ValueError("Oracle did not write a regular PFM prediction")
    with path.open("rb") as source:
        if source.readline(80) != b"Pf\n":
            raise ValueError("Oracle prediction must be grayscale PFM")
        if source.readline(80).strip() != f"{width} {height}".encode():
            raise ValueError("Oracle prediction dimensions differ from prepared inputs")
        scale = float(source.readline(80))
        if not math.isfinite(scale) or scale == 0:
            raise ValueError("Oracle PFM scale is invalid")
        payload = source.read()
    if len(payload) != width * height * 4:
        raise ValueError("Oracle PFM payload length mismatch")
    counts = {"finite": 0, "invalid_positive_infinity": 0}
    byte_order = "<" if scale < 0 else ">"
    for (value,) in struct.iter_unpack(byte_order + "f", payload):
        if math.isinf(value) and value > 0:
            counts["invalid_positive_infinity"] += 1
        elif math.isfinite(value) and 0 <= value < disparities:
            counts["finite"] += 1
        else:
            raise ValueError(f"Oracle disparity outside [0,{disparities}) or +Infinity")
    return counts


def engine_run(name: str, prepared: dict, evaluator: Path, oracle: Path | None, scene_output: Path, timeout: int) -> dict:
    engine_output = scene_output / name
    engine_output.mkdir()
    left, right, truth = (prepared["files"][key] for key in ("left", "right", "truth"))
    ndisp = prepared["common_search_count"]
    calibration = prepared["manifest"].get("calibration")
    calibration_args = (["--fx", str(calibration["fx"]), "--baseline", str(calibration["baseline"]),
                         "--doffs", str(calibration["doffs"])] if calibration else [])
    result = {"engine": name, "search_domain": [0, ndisp - 1],
              "binary_sha256": digest(evaluator if name == "sgbm" else oracle), "steps": []}
    if name == "sgbm":
        score_dir = engine_output / "score"
        command = [str(evaluator), "--left", str(left), "--right", str(right), "--gt", str(truth),
                   *calibration_args, "--ndisp", str(ndisp), "--output-dir", str(score_dir)]
        if "mask" in prepared["files"]:
            command += ["--mask", str(prepared["files"]["mask"])]
        step = execute(command, engine_output / "sgbm.log", timeout)
        result["steps"].append(step)
    else:
        prediction = engine_output / "disparity.pfm"
        command = [str(oracle), "--left", str(left), "--right", str(right), "--ndisp", str(ndisp),
                   "--output", str(prediction)]
        step = execute(command, engine_output / "oracle.log", timeout)
        result["steps"].append(step)
        if step["status"] == "ok" and prediction.is_file():
            result["raw_prediction"] = validate_prediction(
                prediction, prepared["manifest"]["width"], prepared["manifest"]["height"], ndisp)
            score_dir = engine_output / "score"
            score_command = [str(evaluator), "--prediction", str(prediction), "--gt", str(truth),
                             *calibration_args, "--ndisp", str(ndisp), "--output-dir", str(score_dir)]
            if "mask" in prepared["files"]:
                score_command += ["--mask", str(prepared["files"]["mask"])]
            result["steps"].append(execute(score_command, engine_output / "score.log", timeout))
    metrics_path = engine_output / "score/metrics.json"
    result["status"] = "ok" if all(step["status"] == "ok" for step in result["steps"]) and metrics_path.is_file() else "failed"
    if result["status"] == "ok":
        result["metrics"] = json.loads(metrics_path.read_text())
        if name == "sgbm" and result["metrics"].get("sgbm", {}).get("num_disparities") != ndisp:
            result["status"] = "failed"
            result["error"] = "SGBM actual search count differs from common search count"
    result["outputs_sha256"] = {str(path.relative_to(engine_output)): digest(path)
                                 for path in engine_output.rglob("*") if path.is_file()}
    return result


def parse_prepared(value: str) -> tuple[str, Path]:
    name, separator, folder = value.partition("=")
    if not separator or not re.fullmatch(r"[a-zA-Z0-9_-]+", name) or not folder:
        raise argparse.ArgumentTypeError("Use --prepared NAME=DIR")
    return name, Path(folder)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prepared", action="append", required=True, type=parse_prepared)
    parser.add_argument("--elas", required=True, type=Path)
    parser.add_argument("--census", required=True, type=Path)
    parser.add_argument("--evaluator", type=Path, default=DEFAULT_EVALUATOR)
    parser.add_argument("--output-root", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--timeout-seconds", type=int, default=300)
    parser.add_argument("--allow-unpinned", action="store_true", help="for synthetic test fixtures only")
    args = parser.parse_args(argv)
    if args.timeout_seconds < 1 or args.timeout_seconds > 3600:
        parser.error("timeout must be 1–3600 seconds")
    names = [name for name, _ in args.prepared]
    if len(names) != len(set(names)):
        parser.error("prepared scene names must be unique")
    evaluator, elas, census = (path.resolve() for path in (args.evaluator, args.elas, args.census))
    for binary in (evaluator, elas, census):
        if not binary.is_file() or not os.access(binary, os.X_OK):
            parser.error(f"Missing executable: {binary}")
    (ROOT / ".local-tools/tmp").mkdir(parents=True, exist_ok=True)
    pins = source_pins()
    prepared_scenes = [(name, read_prepared(folder, pins, args.allow_unpinned)) for name, folder in args.prepared]
    output_root = args.output_root.resolve()
    output_root.mkdir(parents=True, exist_ok=True)
    if shutil.disk_usage(output_root).free < RESERVE_BYTES + 512 * 1024**2:
        parser.error("Benchmark output filesystem would breach 10 GiB free-space reserve")
    run_folder = output_root / (datetime.now(timezone.utc).strftime("run-%Y%m%dT%H%M%SZ-") + uuid.uuid4().hex[:8])
    run_folder.mkdir()
    summary = {"schema_version": 1, "ok": False, "created_utc": datetime.now(timezone.utc).isoformat(),
               "role": "local benchmark; Census is project-authored, ELAS is external",
               "evaluator": {"path": str(evaluator), "sha256": digest(evaluator),
                             "source_sha256": {str(path.relative_to(ROOT)): digest(path) for path in sorted((ROOT / "core/evaluation").glob("*.cpp"))}},
               "engines": {"elas": {"path": str(elas), "sha256": digest(elas)},
                           "census": {"path": str(census), "sha256": digest(census)}},
               "scenes": {}, "failures": []}
    for name in ("elas", "census"):
        source_folder = ROOT / "scripts/oracles" / name
        summary["engines"][name]["source_sha256"] = {
            str(path.relative_to(ROOT)): digest(path) for path in sorted(source_folder.iterdir()) if path.is_file()
        }
    census_provenance = census.parent / "provenance.json"
    if census_provenance.is_file():
        recorded = json.loads(census_provenance.read_text())
        if recorded.get("binary_sha256") != digest(census) or recorded.get("source_sha256") != digest(ROOT / "scripts/oracles/census/census.cpp"):
            raise ValueError("Census binary/source differs from its build provenance")
        summary["engines"]["census"]["build_provenance"] = recorded
        summary["engines"]["census"]["build_provenance_sha256"] = digest(census_provenance)
    elas_upstream = ROOT / ".local-tools/oracles/elas/upstream"
    elas_provenance = elas.parent / "provenance.json"
    if elas_provenance.is_file():
        recorded = json.loads(elas_provenance.read_text())
        if recorded.get("binary_sha256") != digest(elas) or recorded.get("adapter_sha256") != digest(ROOT / "scripts/oracles/elas/oracle.cpp"):
            raise ValueError("ELAS binary/adapter differs from its build provenance")
        if recorded.get("source_commit") != "862ee4ef30a070753e9b92965855562a4482da05" or recorded.get("git_archive_sha256") != "ae32ab63888994910e6813b65242a8f05ffc86e448048d6fe67dcd1eb4830a8d":
            raise ValueError("ELAS source pin differs from reviewed build")
        summary["engines"]["elas"]["build_provenance"] = recorded
        summary["engines"]["elas"]["build_provenance_sha256"] = digest(elas_provenance)
    if (elas_upstream / ".git").exists():
        commit = subprocess.check_output(["git", "-C", str(elas_upstream), "rev-parse", "HEAD"], text=True).strip()
        if commit != "862ee4ef30a070753e9b92965855562a4482da05":
            raise ValueError("ELAS source checkout differs from reviewed commit")
        dirty = subprocess.check_output(["git", "-C", str(elas_upstream), "status", "--porcelain", "--untracked-files=all"], text=True)
        if dirty:
            raise ValueError("ELAS source checkout has unrecorded changes")
        archive = subprocess.check_output(["git", "-C", str(elas_upstream), "archive", "--format=tar", "HEAD"])
        archive_hash = hashlib.sha256(archive).hexdigest()
        if archive_hash != "ae32ab63888994910e6813b65242a8f05ffc86e448048d6fe67dcd1eb4830a8d":
            raise ValueError("ELAS source archive differs from reviewed pin")
        summary["engines"]["elas"]["upstream_commit"] = commit
        summary["engines"]["elas"]["verified_archive_sha256"] = archive_hash
    try:
        for name, prepared in prepared_scenes:
            scene_output = run_folder / name
            scene_output.mkdir()
            scene = {"prepared": str(prepared["folder"]), "inputs_sha256": prepared["prepared_sha256"],
                     "inputs_manifest_sha256": prepared["manifest_sha256"], "source_provenance": prepared["sources"],
                     "configured_ndisp": prepared["ndisp"], "common_search_count": prepared["common_search_count"],
                     "engines": {}}
            summary["scenes"][name] = scene
            for engine, binary in (("sgbm", None), ("elas", elas), ("census", census)):
                if shutil.disk_usage(run_folder).free < RESERVE_BYTES + 256 * 1024**2:
                    scene["engines"][engine] = {"status": "failed", "error": "10 GiB free-space reserve reached"}
                    summary["failures"].append(f"{name}/{engine}: disk reserve")
                    continue
                try:
                    check_unchanged(prepared)
                    result = engine_run(engine, prepared, evaluator, binary, scene_output, args.timeout_seconds)
                    check_unchanged(prepared)
                    scene["engines"][engine] = result
                    if result["status"] != "ok":
                        summary["failures"].append(f"{name}/{engine}: process failure or missing metrics")
                except Exception as error:  # record each engine failure and continue the matrix
                    scene["engines"][engine] = {"status": "failed", "error": str(error)}
                    summary["failures"].append(f"{name}/{engine}: {error}")
            valid = [record["metrics"] for record in scene["engines"].values() if record["status"] == "ok"]
            if valid and len({(m["roi_pixels"], m["gt_valid"], m["width"], m["height"]) for m in valid}) != 1:
                summary["failures"].append(f"{name}: scored populations differ between engines")
        summary["ok"] = not summary["failures"]
    finally:
        (run_folder / "benchmark.json").write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n")
    print(run_folder / "benchmark.json")
    return 0 if summary["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
