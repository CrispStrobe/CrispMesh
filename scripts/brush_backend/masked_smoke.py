"""Bounded v0.3 Brush smoke on the sealed 60-view masked YCB bridge."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import shutil
import time

from scripts.classical_backend.run import RESERVE, StageError, stage

from .mask_bridge import validate_mask
from .ply import validate as validate_splat_ply
from .preflight import sha256
from .smoke import BINARY, MAX_LOG, MAX_OUTPUT, MAX_RSS, MAX_SECONDS, ROOT, command

MOUNT = Path("/Volumes/backups")
BRIDGE = MOUNT / "code/crispmesh-data/brush-ycb-masked-input-001"
OUTPUT = MOUNT / "code/crispmesh-data/brush-masked-smoke-001"
BRIDGE_SHA256 = "7ef45e2ff8e5997d2079f5af508ef2f4623347cd3ac8335d7e3abc75e260e62d"
BINARY_SHA256 = "8380ed40cce870025393e1ea0257e0752351c67a409d827b76dc75ec3a999a71"
BUFFER = 256 * 1024**2


def verify_input() -> dict:
    report_path = BRIDGE / "bridge-report.json"
    if report_path.is_symlink() or sha256(report_path) != BRIDGE_SHA256:
        raise ValueError("sealed bridge report hash mismatch")
    report = json.loads(report_path.read_text())
    if report.get("schema") != "brush_v030_ycb_mask_bridge_v1" or report.get("status") != "complete":
        raise ValueError("bridge is not complete")
    dataset = BRIDGE / "dataset"
    if dataset.is_symlink() or not dataset.is_dir() or Path(report.get("output_dataset", "")) != dataset:
        raise ValueError("bridge dataset path mismatch")
    expected = ({f"images/{name}" for name in report["source_image_sha256"]} |
                {f"sparse/{name}" for name in report["source_model_sha256"]} |
                {f"masks/{name}" for name in report["masks"]})
    actual = {path.relative_to(dataset).as_posix() for path in dataset.rglob("*") if path.is_file()}
    if expected != actual or any(path.is_symlink() for path in dataset.rglob("*")):
        raise ValueError("bridge file set changed")
    if len(report["source_image_sha256"]) != 60 or len(report["masks"]) != 60:
        raise ValueError("bridge does not cover all 60 views")
    for name, digest in report["source_image_sha256"].items():
        if sha256(dataset / "images" / name) != digest:
            raise ValueError(f"prepared RGB changed: {name}")
    for name, digest in report["source_model_sha256"].items():
        if sha256(dataset / "sparse" / name) != digest:
            raise ValueError(f"prepared camera/model changed: {name}")
    for name, item in report["masks"].items():
        if sha256(dataset / "masks" / name) != item["sha256"]:
            raise ValueError(f"prepared mask changed: {name}")
        validate_mask(dataset / "masks" / name,
                      dataset / "images" / item["registered_image"], tuple(item["size"]))
    return {"bridge_report_sha256": BRIDGE_SHA256,
            "prepared_dataset": str(dataset), "files": len(expected),
            "source_mask_report_sha256": report["source_mask_report_sha256"]}


def preflight() -> dict:
    if not MOUNT.is_mount() or MOUNT.stat().st_dev == ROOT.stat().st_dev:
        raise ValueError("external output mount not distinct from workspace")
    if OUTPUT.exists() or OUTPUT.is_symlink() or OUTPUT.parent.is_symlink() or not OUTPUT.resolve().is_relative_to(MOUNT.resolve()):
        raise ValueError("masked smoke output must be fresh exact external path")
    if BINARY.is_symlink() or sha256(BINARY) != BINARY_SHA256:
        raise ValueError("Brush v0.3 binary changed")
    if shutil.disk_usage(MOUNT).free < RESERVE + MAX_OUTPUT + BUFFER:
        raise ValueError("external 10 GiB floor plus output allowance not met")
    if shutil.disk_usage(ROOT).free < RESERVE + BUFFER:
        raise ValueError("internal 10 GiB floor plus buffer not met")
    inputs = verify_input()
    return {"schema": "brush_masked_smoke_v1", "status": "preflight",
            "inputs": inputs, "binary_sha256": BINARY_SHA256,
            "command": command(BINARY, BRIDGE / "dataset", OUTPUT),
            "limits": {"seconds": MAX_SECONDS, "output_bytes": MAX_OUTPUT,
                       "log_bytes": MAX_LOG, "sampled_process_rss_bytes": MAX_RSS,
                       "external_free_bytes_floor": RESERVE,
                       "internal_free_bytes_floor": RESERVE},
            "camera_lane": "image_estimated", "artifact_type": "splats",
            "mask_semantics": "0 ignore / 255 keep RGB loss; coarse photo-derived pose support",
            "reference_scope": "none", "quality_claim": False}


def run() -> dict:
    report = preflight()
    OUTPUT.mkdir()
    start = time.monotonic()
    failure = None
    try:
        report["stage"] = stage(OUTPUT, "train", report["command"], start + MAX_SECONDS,
                                MAX_OUTPUT, MAX_LOG, MAX_RSS,
                                extra_reserve_paths=(ROOT,))
    except StageError as exc:
        report["stage"] = exc.result
        failure = str(exc)
    except Exception as exc:
        failure = f"{type(exc).__name__}: {exc}"
    try:
        if verify_input() != report["inputs"] or sha256(BINARY) != BINARY_SHA256:
            failure = failure or "prepared input or binary changed during training"
    except Exception as exc:
        failure = failure or f"postflight source check: {exc}"
    exports = sorted(OUTPUT.glob("export_*.ply"))
    validated = {}
    for path in exports:
        try:
            validated[path.name] = {**validate_splat_ply(path), "sha256": sha256(path)}
        except Exception as exc:
            failure = failure or f"invalid splat export: {exc}"
    if not exports:
        failure = failure or "no Brush splat PLY export"
    if shutil.disk_usage(MOUNT).free < RESERVE or shutil.disk_usage(ROOT).free < RESERVE:
        failure = failure or "postflight internal/external 10 GiB floor"
    report.update(status="failed" if failure else "complete", failure=failure,
                  elapsed_seconds=round(time.monotonic() - start, 3),
                  exports=validated, quality_claim=False)
    with (OUTPUT / "report.json").open("x") as stream:
        json.dump(report, stream, indent=2, sort_keys=True)
        stream.write("\n")
    return report


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preflight", action="store_true")
    args = parser.parse_args()
    report = preflight() if args.preflight else run()
    print(json.dumps({key: report[key] for key in ("status", "command", "inputs")}, indent=2))
    if report["status"] == "failed":
        raise SystemExit(1)


if __name__ == "__main__":
    main()
