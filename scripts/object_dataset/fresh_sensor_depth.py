"""Frozen evaluation-only Berkeley depth diagnostic for fresh OpenMVS 005."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import shutil
import time

import numpy as np

from scripts.classical_backend import geometry, score_fresh_complete
from scripts.object_dataset import evaluate, sensor_depth as sd


RUN = Path("/Volumes/backups/code/crispmesh-data/turntable-fresh-openmvs-005")
CAMERA_FIT = Path("/Volumes/backups/code/crispmesh-data/fresh-sensor-depth-005-preflight/camera-fit.json")
BASELINE = sd.ROOT / "build-opencv/sensor-depth-004/report.json"
RESULT_SHA = "dafc6ccb2a90f0c5a45748291571455237acd8ae08da01dd80adc10222a3625e"
CAMERA_FIT_SHA = "d9e51355eb7776132d8abba83d5d2b8be8ba2d6c073149b9d1d9d7a156efc3b6"
BASELINE_SHA = "9027600ccac5488ec422a6adc3736979eb126a7b33d9d8d19d851b107327cc5c"
RAY_SHA = (
    "3944d636194653b6d6c832dfd91262a6231ece210050583fec2d1641c2b45f68",
    "6cdc1c81278dcf8302be4bcb3cbd9f59d1121ecdd1a9f4db357542dc117d5c7d",
    "0cf8f4aaacda2785d4aa5b78f95313e94dcb0d3e5623c8d547e64873319fabaf",
)
INTERIOR_COUNTS = (1622, 1107, 1058)
MAX_REPORT_BYTES = 20_000_000


def disk_floor(output: Path) -> dict[str, int]:
    free = {str(sd.ROOT): shutil.disk_usage(sd.ROOT).free,
            str(output.parent): shutil.disk_usage(output.parent).free}
    if min(free.values()) < sd.MIN_FREE_BYTES:
        raise OSError("both workspace and backup volumes require at least 10 GiB free")
    return free


def matrix_from_fit(fit: dict, expected_model: dict) -> np.ndarray:
    if (fit.get("schema") != "ycb_stock_berkeley_camera_diagnostic_v1" or
            fit.get("candidate_model_sha256") != expected_model or
            fit.get("reference_evidence", {}).get("report_sha256") !=
            "744b50dfcbf66c0801fa9f36c3a7eac124ba09f3053909c10e7c647d585a723e"):
        raise ValueError("fresh camera fit is not bound to exact source/rig")
    comparison = fit["comparison"]
    if (comparison.get("paired_camera_count") != 60 or
            comparison.get("missing_reference_names") or
            comparison.get("extra_candidate_names") or
            comparison["center_rms_over_reference_radius"] >= .05 or
            comparison["orientation_p95_degrees"] >= 10):
        raise ValueError("camera-only similarity misses frozen fit gate")
    sim = comparison["similarity_source_to_reference"]
    matrix = np.eye(4)
    matrix[:3, :3] = sim["scale"] * np.asarray(sim["rotation"], dtype=float)
    matrix[:3, 3] = sim["translation"]
    return sd.proper_similarity(matrix)


def same_rays(frames: list[dict], baseline: dict) -> None:
    if (baseline.get("schema") != "berkeley_sensor_depth_v1" or
            baseline.get("status") != "complete" or len(baseline.get("frames", [])) != 3):
        raise ValueError("prior comparison is not complete")
    for i, (frame, old) in enumerate(zip(frames, baseline["frames"])):
        if (frame["angle"] != old["angle"] or
                frame["selection"]["selected_sha256"] != RAY_SHA[i] or
                frame["selection"] != old["selection"] or
                frame["selected_indices"] != old["selected_indices"] or
                frame["selected_interior_count"] != INTERIOR_COUNTS[i]):
            raise ValueError("selected rays or support differ from frozen baseline")
        old_candidate = baseline["candidates"][0]["frames"][i]
        if (not np.array_equal(frame["observed"], old_candidate["observed_m"]) or
                frame["interior"].tolist() != old_candidate["interior_member"]):
            raise ValueError("observed depths or interior membership differ")


def write_report(path: Path, report: dict) -> None:
    encoded = (json.dumps(report, indent=2, allow_nan=False) + "\n").encode()
    if len(encoded) > MAX_REPORT_BYTES:
        raise ValueError("fresh sensor-depth report exceeds 20 MB")
    disk_floor(path)
    path.write_bytes(encoded)


def evaluate_run(output: Path) -> dict:
    output = Path(output)
    if (output.exists() or output.is_symlink() or not output.parent.is_dir() or
            output.parent.is_symlink() or not str(output.resolve()).startswith(
                "/Volumes/backups/code/crispmesh-data/")):
        raise ValueError("output must be a fresh directory on the backup data volume")
    before_free = disk_floor(output)
    if (sd.sha256(RUN / "result.json") != RESULT_SHA or
            sd.sha256(CAMERA_FIT) != CAMERA_FIT_SHA or sd.sha256(BASELINE) != BASELINE_SHA):
        raise ValueError("sealed fresh producer, camera fit, or baseline changed")
    evidence = score_fresh_complete.checked_run(RUN)
    fit = json.loads(CAMERA_FIT.read_text())
    matrix = matrix_from_fit(fit, evidence["sparse_sha256"])
    gauge = sd.verify_dense_camera_gauge(RUN / "dense/sparse", RUN / "sparse/0")
    reference = json.loads(sd.REFERENCE_003.read_text())
    projection = json.loads(sd.PROJECTION_002.read_text())
    deadline = time.monotonic() + sd.MAX_SECONDS
    frames = sd._prepared_frames(reference, projection, deadline)
    baseline = json.loads(BASELINE.read_text())
    same_rays(frames, baseline)
    stages = json.loads((RUN / "result.json").read_text())["stages"]
    stage_faces = {stage["name"]: stage["artifact"]["faces"] for stage in stages
                   if stage["name"] in ("mesh", "refine") and stage["status"] == "complete"}
    if set(stage_faces) != {"mesh", "refine"}:
        raise ValueError("rough and refined producer stages must both be complete")
    critical = [RUN / "result.json", CAMERA_FIT, BASELINE, sd.REFERENCE_003, sd.PROJECTION_002,
                sd.CALIBRATION, RUN / "mesh.ply", RUN / "refined.ply"]
    critical += [RUN / "sparse/0" / name for name in evidence["sparse_sha256"]]
    critical += [RUN / "dense/sparse" / name for name in evidence["sparse_sha256"]]
    before = {str(path): sd.sha256(path) for path in critical}
    output.mkdir()
    result = {"schema": "fresh_berkeley_sensor_depth_v1", "status": "in_progress",
              "scope": "evaluation-only; rough primary, refined secondary; no mesh or pose residual fitting",
              "protocol": {"angles": list(sd.ANGLES), "max_rays_per_view": sd.MAX_RAYS_PER_VIEW,
                           "seed_base": 20260927, "interior_erosion_rgb_pixels": 16,
                           "interval_m": [sd.NEAR_METRES, sd.FAR_METRES],
                           "deadline_seconds": sd.MAX_SECONDS},
              "inputs_sha256": before, "producer": evidence,
              "camera_fit": {"sha256": CAMERA_FIT_SHA, "comparison": fit["comparison"],
                             "world_to_table_matrix": matrix.tolist(), "dense_gauge": gauge},
              "disk_free_before_bytes": before_free,
              "frames": [{key: value for key, value in frame.items()
                          if key not in ("observed", "rays", "interior", "ir_from_table")}
                         for frame in frames], "candidates": []}
    report_path = output / "report.json"
    write_report(report_path, result)
    try:
        for label, name, stage in (("rough005", "mesh.ply", "mesh"),
                                   ("refined005", "refined.ply", "refine")):
            if time.monotonic() > deadline:
                raise TimeoutError("fresh sensor-depth 300-second deadline")
            mesh = RUN / name
            if before[str(mesh)] != json.loads((RUN / "result.json").read_text())["artifact_sha256"][name]:
                raise ValueError("native mesh differs from sealed producer")
            converted_path = output / f"{label}-normalized-geometry.ply"
            conversion = geometry.export_geometry(mesh, converted_path)
            info, vertices, faces = evaluate.inspect_ply(converted_path, geometry=True)
            if (info["nontriangle_faces"] or info["faces"] != stage_faces[stage] or
                    conversion["faces"] != stage_faces[stage] or
                    conversion["source_sha256"] != before[str(mesh)]):
                raise ValueError("normalized mesh differs from completed native stage")
            table_vertices = sd.transform_vertices(vertices, matrix)
            rows, observed_all, predicted_all, interior_all = [], [], [], []
            for frame in frames:
                ir_vertices = sd.transform_vertices(table_vertices, frame["ir_from_table"])
                predicted, degenerate = sd.first_hit_depths(
                    ir_vertices, faces, frame["rays"], deadline=deadline)
                observed, interior = frame["observed"], frame["interior"]
                observed_all.append(observed)
                predicted_all.append(predicted)
                interior_all.append(interior)
                rows.append({"angle": frame["angle"], "excluded_zero_area_faces": degenerate,
                             "observed_m": observed.tolist(),
                             "predicted_m": sd.nullable_depths(predicted),
                             "interior_member": interior.tolist(),
                             "coarse": sd.residual_summary(observed, predicted),
                             "interior": sd.residual_summary(observed, predicted, interior)})
            observed = np.concatenate(observed_all)
            predicted = np.concatenate(predicted_all)
            interior = np.concatenate(interior_all)
            result["candidates"].append({"label": label, "stage": stage,
                "native_mesh_sha256": before[str(mesh)], "normalization": conversion,
                "mesh_info": info, "frames": rows,
                "pooled": {"coarse": sd.residual_summary(observed, predicted),
                           "interior": sd.residual_summary(observed, predicted, interior)}})
            write_report(report_path, result)
        result["prior_rough"] = [{"label": row["label"], "stage": row["stage"],
                                  "pooled": row["pooled"],
                                  "frames": [{"angle": frame["angle"], "coarse": frame["coarse"],
                                              "interior": frame["interior"]} for frame in row["frames"]]}
                                 for row in baseline["candidates"]]
        if {row["label"] for row in result["prior_rough"]} != {"rough008", "rough014", "rough015"}:
            raise ValueError("unexpected prior rough comparison inventory")
        if {str(path): sd.sha256(path) for path in critical} != before:
            raise ValueError("critical inputs changed during score")
        score_fresh_complete.checked_run(RUN)
        result["disk_free_after_bytes"] = disk_floor(output)
        result["status"] = "complete"
    except Exception as error:
        result["status"] = "failed"
        result["failure"] = {"type": type(error).__name__, "message": str(error)}
        write_report(report_path, result)
        raise
    write_report(report_path, result)
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--approve-assumption-qualified-gate", action="store_true")
    args = parser.parse_args()
    if not args.approve_assumption_qualified_gate:
        parser.error("live comparison requires explicit approved qualified gate")
    report = evaluate_run(args.output)
    print(json.dumps({"status": report["status"], "output": str(args.output / "report.json"),
                      "candidates": [row["label"] for row in report["candidates"]]}))


if __name__ == "__main__":
    main()
