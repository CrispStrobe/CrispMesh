"""Frozen, bounded post hoc Apple bunny mesh evaluation against scan and ROI.

Default is read-only preflight. --execute runs the unchanged bunny fit/score
programs only after all source hashes and reference thresholds pass.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time

from scripts.classical_backend.run import digest, folder_bytes


REPO = Path(__file__).resolve().parents[2]
APPLE = Path("/Volumes/backups/code/crispmesh-data/apple-bunny73-preview-001-output/model.usdz")
APPLE_SHA = "2dbefc3216132134124068fdceb9cf4eb43e5d922c59b47db750086042915d60"
LAUNCH = APPLE.with_name("result.json")
MESH_REVIEW = Path("/Volumes/backups/code/crispmesh-data/apple-bunny73-review-001")
MESH = MESH_REVIEW / "mesh-untextured.ply"
MESH_SHA = "41a6de35cad340f805cec567e2c69f12eed9691f8ae7672d664814756f661303"
FULL = REPO / ".local-tools/test-data/3dlf-scan-bunny/revopoint/bunny/fuse_mesh_rgb.ply"
FULL_SHA = "28ed4462b6ee84edcc87157b49aee9216ef170ddd1e176928c72c74233ef9520"
ROI = REPO / "build-opencv/bunny-scanner-roi-v1/reference-ygtminus24.ply"
ROI_SHA = "113c1a230a88559760fb7dbedf3e9e09fda9b099ebb4bf11953d5c860d33accc"
ROI_RECEIPT = ROI.with_name("export-report.json")
ALIGN = REPO / "scripts/object_dataset/align.py"
ALIGN_SHA = "24f27a0e2ff8879e5635151f73340b4abebbc04276f115cce627fec515f37b64"
SCORE = REPO / "scripts/object_dataset/surface_metrics.py"
SCORE_SHA = "466541e870f51411db647f96e1f1060e7c574ededdb9d92ded81b4ce43bebb16"
PYTHON = REPO / ".local-tools/colmap-sparse/venv/bin/python"
OUTPUT = REPO / "build-opencv/apple-bunny73-evaluation-001"
FULL_THRESHOLDS = (1.0429398885, 2.085879777, 4.171759554)
ROI_THRESHOLDS = (0.6140926143, 1.2281852285, 2.4563704570)
FLOOR = 11 << 30
OUTPUT_CAP = 8 << 20
LOG_CAP = 1 << 20
SCHEMA = "apple_bunny73_frozen_scan_evaluation_v1"


def _sealed(path: Path, expected: str) -> str:
    if path.is_symlink() or not path.is_file() or digest(path) != expected:
        raise ValueError(f"sealed input missing or changed: {path}")
    return expected


def commands(output: Path, python: Path = PYTHON) -> list[tuple[str, list[str], Path, int]]:
    stages = []
    for name, reference, thresholds, object_id in (
        ("whole", FULL, FULL_THRESHOLDS, "3dlf-scan:bunny"),
        ("roi", ROI, (*ROI_THRESHOLDS, *FULL_THRESHOLDS), "bunny-roi-posthoc"),
    ):
        fit = output / f"{name}-reference-fit.json"
        metrics = output / f"{name}-surface-metrics.json"
        stages.append((f"{name}_fit", [str(python), str(ALIGN),
                       "--reference", str(reference), "--output", str(MESH),
                       "--object-id", object_id,
                       "--reference-units", "Revopoint scanner coordinate units; physical scale unverified",
                       "--save-transform", str(fit), "--samples", "1024"], fit, 75))
        command = [str(python), str(SCORE), "--reference", str(reference),
                   "--output", str(MESH), "--transform", str(fit)]
        for threshold in thresholds:
            command.extend(("--threshold", str(threshold)))
        command.extend(("--samples", "4096", "--seed", "2027",
                        "--save-report", str(metrics)))
        stages.append((f"{name}_score", command, metrics, 330))
    return stages


def preflight(output: Path = OUTPUT, python: Path = PYTHON) -> dict:
    if output.exists() or output.is_symlink() or not output.parent.is_dir():
        raise ValueError("evaluation output must be a fresh directory")
    hashes = {"apple_usdz": _sealed(APPLE, APPLE_SHA),
              "apple_mesh": _sealed(MESH, MESH_SHA),
              "whole_scan": _sealed(FULL, FULL_SHA),
              "scanner_roi": _sealed(ROI, ROI_SHA),
              "align_script": _sealed(ALIGN, ALIGN_SHA),
              "surface_metrics_script": _sealed(SCORE, SCORE_SHA)}
    if not python.is_file() or not python.resolve().is_file():
        raise ValueError("project Python environment missing")
    for source in (LAUNCH, MESH_REVIEW / "result.json", ROI_RECEIPT):
        if source.is_symlink() or not source.is_file():
            raise ValueError(f"missing or linked receipt: {source}")
    launch = json.loads(LAUNCH.read_text())
    mesh_review = json.loads((MESH_REVIEW / "result.json").read_text())
    roi = json.loads(ROI_RECEIPT.read_text())
    if (launch.get("status") != "complete" or launch.get("mode") != "rgb_to_usdz" or
            launch.get("artifact", {}).get("sha256") != APPLE_SHA or
            launch.get("source_images_unchanged") is not True or
            mesh_review.get("source_usdz_sha256") != APPLE_SHA or
            mesh_review.get("ply", {}).get("sha256") != MESH_SHA or
            roi.get("reference_sha256") != FULL_SHA or roi.get("roi_sha256") != ROI_SHA or
            roi.get("rule") != "retain triangle iff all three scanner vertices have Y > -24; no cap; no candidate selection"):
        raise ValueError("Apple or scanner ROI receipt does not bind the frozen mesh/reference")
    if (shutil.disk_usage(output.parent).free < FLOOR + OUTPUT_CAP or
            shutil.disk_usage(APPLE).free < FLOOR):
        raise ValueError("11 GiB free-space floor or 8 MiB output headroom unavailable")
    return {"schema": SCHEMA, "status": "read_only_preflight",
            "scope": "post_hoc_reference_fitted_diagnostic; Apple .preview macOS-only; no ranking",
            "hashes": hashes,
            "receipt_sha256": {str(path): digest(path) for path in
                               (LAUNCH, MESH_REVIEW / "result.json", ROI_RECEIPT)},
            "python": str(python), "python_sha256": digest(python),
            "output": str(output), "commands": [row[1] for row in commands(output, python)],
            "settings": {"fit_samples": 1024, "fit_seed": 2026,
                         "score_samples_each_direction": 4096, "score_seed": 2027,
                         "whole_thresholds": FULL_THRESHOLDS,
                         "roi_thresholds": ROI_THRESHOLDS,
                         "roi_original_absolute_thresholds": FULL_THRESHOLDS,
                         "output_cap_bytes": OUTPUT_CAP, "disk_floor_bytes": FLOOR}}


def _stage(name: str, command: list[str], artifact: Path, limit: int,
           output: Path) -> dict:
    env = os.environ.copy()
    env.update(OPENBLAS_NUM_THREADS="2", OMP_NUM_THREADS="2", PYTHONPATH=str(REPO))
    log_path = output / f"{name}.log"
    started = time.monotonic()
    reason = None
    with log_path.open("wb") as log:
        process = subprocess.Popen(command, cwd=REPO, env=env, stdout=log,
                                   stderr=subprocess.STDOUT, start_new_session=True)
        try:
            while process.poll() is None:
                if time.monotonic() - started > limit:
                    reason = "stage deadline exceeded"
                elif (folder_bytes(output) > OUTPUT_CAP or log_path.stat().st_size > LOG_CAP or
                      shutil.disk_usage(output).free < FLOOR or
                      shutil.disk_usage(APPLE).free < FLOOR):
                    reason = "output, log, or disk floor reached"
                if reason:
                    os.killpg(process.pid, signal.SIGTERM)
                    break
                time.sleep(0.5)
        finally:
            if process.poll() is None:
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
    code = process.wait()
    if code != 0 or not artifact.is_file() or artifact.is_symlink():
        reason = reason or f"stage exited {code} or produced no sealed report"
    if folder_bytes(output) > OUTPUT_CAP:
        reason = reason or "output cap reached after stage"
    return {"name": name, "status": "complete" if reason is None else "failed",
            "reason": reason, "returncode": code, "seconds": time.monotonic() - started,
            "command": command, "log": str(log_path),
            "artifact": str(artifact),
            "artifact_sha256": digest(artifact) if reason is None else None}


def execute(output: Path = OUTPUT, python: Path = PYTHON) -> dict:
    report = preflight(output, python)
    output.mkdir()
    report["status"] = "running"
    report["stages"] = []
    receipt = output / "result.json"

    def save() -> None:
        receipt.write_text(json.dumps(report, indent=2) + "\n")

    save()
    try:
        for name, command, artifact, limit in commands(output, python):
            if (digest(APPLE) != APPLE_SHA or digest(MESH) != MESH_SHA or
                    digest(FULL) != FULL_SHA or digest(ROI) != ROI_SHA):
                raise ValueError("mesh or reference changed before evaluation stage")
            result = _stage(name, command, artifact, limit, output)
            report["stages"].append(result)
            save()
            if result["status"] != "complete":
                raise RuntimeError(f"{name}: {result['reason']}")
            data = json.loads(artifact.read_text())
            reference_hash = FULL_SHA if name.startswith("whole") else ROI_SHA
            if (data.get("reference_sha256") != reference_hash or
                    data.get("output_sha256") != MESH_SHA or
                    data.get("registration_basis") != "reference-fit"):
                raise ValueError(f"{name} report not bound to exact scan and Apple mesh")
        report["status"] = "complete_conditional_diagnostics_only"
    except Exception as error:
        report.update(status="failed", failure=str(error))
    report["output_bytes"] = folder_bytes(output)
    report["source_unchanged"] = (digest(APPLE) == APPLE_SHA and digest(MESH) == MESH_SHA and
                                   digest(FULL) == FULL_SHA and digest(ROI) == ROI_SHA)
    report["free_bytes_after"] = {str(output): shutil.disk_usage(output).free,
                                  str(APPLE): shutil.disk_usage(APPLE).free}
    if (not report["source_unchanged"] or report["output_bytes"] > OUTPUT_CAP or
            any(value < FLOOR for value in report["free_bytes_after"].values())):
        report.update(status="failed", failure="source or resource postcheck failed")
    save()
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--execute", action="store_true")
    args = parser.parse_args()
    report = execute() if args.execute else preflight()
    print(json.dumps({"status": report["status"], "output": report["output"],
                      "failure": report.get("failure")}))
    return 1 if report["status"] == "failed" else 0


if __name__ == "__main__":
    raise SystemExit(main())
