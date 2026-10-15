"""Read-only TRAIN-photo diagnostic for the sealed OpenMVG mustard sparse run.

Uses only the supervisor receipt, OpenMVG HTML report and binary PLY. PLY
camera names are inferred from shared writer iteration order, never from a
stored PLY ID. Ambiguous order or coverage causes an explicit abstention.
"""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
import re
import statistics

from scripts.classical_backend import openmvg_sceaux_postcheck as shared


OUTPUT = Path("/Volumes/backups/code/crispmesh-data/openmvg-mustard-photo-sfm-001")
SCHEMA = "openmvg_mustard_train48_photo_control_v1"
STAGES = ("listing", "features", "putative", "geometric", "sfm")
TRAIN_NAMES_SHA = "a81d1647109db27991c39c5fe1a9ae308c04081f8ada49d8b0f5ab1c15245544"
STAGE_REPORT_SHA = "bbcba624f6a51f82e0ebdebf6e7e4cda10804bc8df34cfd0d5b3242983a225d0"


def percentile(values: list[float], q: float) -> float:
    ordered = sorted(values)
    if not ordered:
        raise ValueError("empty distance distribution")
    position = (len(ordered) - 1) * q
    lo = math.floor(position)
    hi = math.ceil(position)
    return ordered[lo] * (hi - position) + ordered[hi] * (position - lo) if lo != hi else ordered[lo]


def report_rows(path: Path, names: tuple[str, ...]) -> dict:
    if path.is_symlink() or not path.is_file() or not (0 < path.stat().st_size <= shared.REPORT_LIMIT):
        raise ValueError("missing, linked, empty, or oversized OpenMVG report")
    html = path.read_text(encoding="utf-8", errors="replace")
    counts = {}
    for key in ("views", "poses", "intrinsics", "tracks", "residuals"):
        hits = re.findall(r"#" + key + r":\s*(\d+)\s*<br\s*/?>", html)
        if len(hits) != 1:
            raise ValueError(f"report lacks unique #{key}")
        counts[key] = int(hits[0])
    tables = shared.Tables()
    tables.feed(html)
    header = ["IdView", "Basename", "#Observations", "Residuals min",
              "Residuals median", "Residuals mean", "Residuals max"]
    at = [i for i, row in enumerate(tables.rows) if row == header]
    if len(at) != 1:
        raise ValueError("report lacks unique per-view table")
    stems = {Path(name).stem: name for name in names}
    if len(stems) != 48 or any(not re.fullmatch(r"NP3_\d{3}\.jpg", n) for n in names):
        raise ValueError("sealed TRAIN names are malformed or duplicated")
    rows = tables.rows[at[0] + 1: at[0] + 49]
    if len(rows) != 48:
        raise ValueError("report lacks 48 per-view rows")
    viewed = []
    for row in rows:
        if len(row) not in (2, 7) or row[1] not in stems:
            raise ValueError("report row has malformed or unknown TRAIN stem")
        try:
            view_id = int(row[0])
        except ValueError as exc:
            raise ValueError("report view ID is malformed") from exc
        entry = {"id": view_id, "name": stems[row[1]], "angle_deg": int(row[1][4:])}
        if len(row) == 7:
            try:
                entry["observations"] = int(row[2])
                values = [float(x) for x in row[3:]]
            except ValueError as exc:
                raise ValueError("report observations/residuals malformed") from exc
            if (entry["observations"] <= 0 or any(not math.isfinite(x) or x < 0 for x in values) or
                    not (values[0] <= values[1] <= values[3] and values[0] <= values[2] <= values[3])):
                raise ValueError("report observations/residuals invalid")
            entry.update(min_px=values[0], median_px=values[1], mean_px=values[2], max_px=values[3])
        viewed.append(entry)
    observed = [row for row in viewed if "observations" in row]
    if (len({row["name"] for row in viewed}) != 48 or len({row["id"] for row in viewed}) != 48 or
            counts["views"] != 48 or counts["poses"] != 46 or counts["intrinsics"] <= 0 or
            counts["tracks"] <= 0 or counts["residuals"] <= 0 or
            sum(row["observations"] for row in observed) != counts["residuals"]):
        raise ValueError("report coverage/counts inconsistent")
    rmse_hits = re.findall(r"SfM Scene RMSE:\s*([+\-\d.eE]+)", html)
    if len(rmse_hits) != 1 or not math.isfinite(float(rmse_hits[0])) or float(rmse_hits[0]) < 0:
        raise ValueError("report RMSE missing or nonfinite")
    return {"counts": counts, "rows": viewed, "observed_rows": observed,
            "missing_names": [row["name"] for row in viewed if "observations" not in row],
            "rmse_px_per_axis": float(rmse_hits[0])}


def ply_centers(path: Path) -> tuple[dict, list[tuple[float, float, float]]]:
    summary = shared.parse_ply(path)
    # Pinned writer emits all posed-view green points first, then white landmarks.
    with path.open("rb") as stream:
        for _ in range(13):
            line = stream.readline(256)
            if not line:
                raise ValueError("PLY header ended before payload")
            if line.strip() == b"end_header":
                break
        else:
            raise ValueError("PLY header exceeds pinned writer layout")
        centers = []
        for index in range(summary["vertices"]):
            x, y, z, r, g, b = shared.PLY_VERTEX.unpack(stream.read(shared.PLY_VERTEX.size))
            if index < summary["colors"]["camera_centers"]:
                if (r, g, b) != (0, 255, 0):
                    raise ValueError("PLY camera block order differs from pinned writer")
                centers.append((x, y, z))
            elif (r, g, b) != (255, 255, 255):
                raise ValueError("PLY structure block differs from pinned writer")
    return summary, centers


def orbit_diagnostic(angles_to_centers: dict[int, tuple[float, float, float]]) -> dict:
    angles = sorted(angles_to_centers)
    if len(angles) != 46 or angles[0] != 0 or angles[-1] != 348:
        raise ValueError("angular diagnostic needs 46 expected TRAIN angles")
    distances = {6: [], 12: [], 180: []}
    for i, angle in enumerate(angles):
        for next_angle in angles[i + 1:]:
            gap = min(next_angle - angle, 360 - (next_angle - angle))
            if gap in distances:
                distances[gap].append(math.dist(angles_to_centers[angle], angles_to_centers[next_angle]))
    if any(not values or any(not math.isfinite(x) or x <= 0 for x in values)
           for values in distances.values()):
        raise ValueError("degenerate adjacent or opposing camera centers")
    close = math.dist(angles_to_centers[0], angles_to_centers[348])
    twelve_without_close = [math.dist(angles_to_centers[a], angles_to_centers[(a + 12) % 360])
                            for a in angles if (a + 12) % 360 in angles and a != 348]
    if not twelve_without_close:
        raise ValueError("no independent 12-degree comparison pairs")
    six, twelve, opposite = (distances[k] for k in (6, 12, 180))
    median_six, median_opp = statistics.median(six), statistics.median(opposite)
    close_ratio = close / statistics.median(twelve_without_close)
    adjacent_ratio = median_six / median_opp
    opp_spread = percentile(opposite, .90) / percentile(opposite, .10)
    return {"mapping": "provisional source-order inference: observed HTML rows to green PLY points; PLY has no view IDs",
            "registered_angles_deg": angles,
            "max_registered_angular_gap_deg": max((angles[(i + 1) % len(angles)] - angle) % 360
                                                  for i, angle in enumerate(angles)),
            "six_degree_pairs": len(six), "twelve_degree_pairs": len(twelve),
            "opposing_180_degree_pairs": len(opposite),
            "median_six_degree_chord": median_six,
            "median_twelve_degree_chord": statistics.median(twelve),
            "median_opposing_chord": median_opp,
            "six_degree_chord_over_opposing": adjacent_ratio,
            "ideal_circle_six_degree_ratio": math.sin(math.radians(3)),
            "opposing_p90_over_p10": opp_spread,
            "full_turn_closure_000_to_348_chord": close,
            "full_turn_closure_over_other_12deg_median": close_ratio,
            "diagnostic_flags": {"closure_within_2x_other_12deg": close_ratio <= 2,
                                 "sixdeg_over_opposing_between_0_01_and_0_2": .01 <= adjacent_ratio <= .2,
                                 "opposing_p90_over_p10_at_most_2": opp_spread <= 2}}


def diagnose(output: Path = OUTPUT, *, expected_root: Path = OUTPUT) -> dict:
    if output.resolve() != expected_root.resolve() or output.is_symlink() or not output.is_dir():
        raise ValueError("diagnostic accepts only the sealed mustard -001 output root")
    receipt_path = output / "receipt.json"
    if receipt_path.is_symlink() or not receipt_path.is_file() or receipt_path.stat().st_size > shared.REPORT_LIMIT:
        raise ValueError("missing, linked, or oversized mustard receipt")
    receipt_hash = shared.sha256(receipt_path)
    receipt = json.loads(receipt_path.read_text())
    names = tuple(receipt.get("photos", {}).get("names", []))
    photos = receipt.get("photos", {})
    stages = receipt.get("stages", [])
    if (receipt.get("schema") != SCHEMA or receipt.get("output") != str(output.resolve()) or
            receipt.get("status") != "failed_registration_or_sparse_gate" or
            photos.get("train_names_sha256") != TRAIN_NAMES_SHA or
            photos.get("stage_report_sha256") != STAGE_REPORT_SHA or
            len(names) != 48 or len(stages) != 5 or
            [s.get("name") for s in stages] != list(STAGES) or
            any(s.get("status") != "completed" for s in stages)):
        raise ValueError("mustard photo-control receipt is not the completed sparse attempt")
    inventory = receipt.get("output_inventory", {})
    hashes = {}
    for relative in (shared.REPORT, shared.PLY, shared.MODEL):
        path, row = output / relative, inventory.get(relative)
        if (path.is_symlink() or not path.is_file() or not isinstance(row, dict) or
                path.stat().st_size != row.get("bytes") or shared.sha256(path) != row.get("sha256")):
            raise ValueError(f"mustard sparse artifact seal differs: {relative}")
        hashes[relative] = row["sha256"]
    if stages[-1].get("artifact_sha256") != hashes[shared.MODEL]:
        raise ValueError("SfM model stage hash differs from receipt")
    report = report_rows(output / shared.REPORT, names)
    if receipt.get("sfm_report") != report["counts"]:
        raise ValueError("report counts differ from supervisor receipt")
    ply, centers = ply_centers(output / shared.PLY)
    if (ply["colors"]["camera_centers"] != report["counts"]["poses"] or
            ply["colors"]["landmarks"] != report["counts"]["tracks"] or
            ply["colors"]["pose_priors"] or ply["colors"]["control_points"]):
        raise ValueError("PLY/report camera-landmark counts differ")
    result = {"schema": "openmvg_mustard_sparse_diagnostic_v1",
              "status": "partial_sparse_diagnostic", "artifact_sha256": hashes,
              "source_receipt_sha256": receipt_hash,
              "counts": report["counts"], "missing_names": report["missing_names"],
              "per_view": report["rows"], "rmse_px_per_axis": report["rmse_px_per_axis"],
              "ply": ply, "orbit": {"status": "abstained", "reason": "camera order cannot be mapped"},
              "unverified": ["cereal intrinsics/rotations/track membership",
                             "independently recomputed reprojections", "reference agreement"]}
    # Equality proves each posed view has a residual row: source emits residual
    # rows only for posed views, and green PLY points only for posed views.
    if len(report["observed_rows"]) == report["counts"]["poses"] == len(centers):
        angle_centers = {row["angle_deg"]: center
                         for row, center in zip(report["observed_rows"], centers)}
        if len(angle_centers) == len(centers):
            result["orbit"] = {"status": "provisional_source_order", **orbit_diagnostic(angle_centers)}
    for relative, sha in hashes.items():
        if shared.sha256(output / relative) != sha:
            raise ValueError(f"mustard sparse artifact changed during diagnostic: {relative}")
    if shared.sha256(receipt_path) != receipt_hash:
        raise ValueError("mustard receipt changed during diagnostic")
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=OUTPUT)
    args = parser.parse_args()
    print(json.dumps(diagnose(args.output), sort_keys=True))


if __name__ == "__main__":
    main()
