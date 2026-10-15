#!/usr/bin/env python3
"""Bounded, TRAIN-only checkerboard-assisted camera-pose diagnostic.

The board moves with the bottle. Its unmarked corner labels have a global 180°
ambiguity, and square units are arbitrary. No reference pose enters this code.
"""

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

os.environ.setdefault("OPENCV_OPENCL_RUNTIME", "disabled")
os.environ.setdefault("OPENBLAS_NUM_THREADS", "2")
import cv2
import numpy as np

from scripts.object_motion.orbit_plausibility import evaluate as orbit_evaluate
from scripts.object_motion.orbit_plausibility import validate_profile


ROOT = Path(__file__).resolve().parents[2]
MANIFEST = Path("/Volumes/backups/code/crispmesh-data/mustard-feature-mask-pair-001-sam/inputs.json")
MANIFEST_SHA256 = "bc8d06024627219736959c7b4a34173cdd918bb5fba666528f08ca4a7a1957ae"
TRAIN_IMAGES = Path("/Volumes/backups/code/crispmesh-data/mustard-sfm-train-001/images")
PROFILE = ROOT / "tests/datasets/ycb_np3_full_turn_profile.json"
PROFILE_SHA256 = "bb00d17e1940f121dc12f7ddb31d4adf80b7e9a935f18e5535cfc1a2cde1998d"
EXTERNAL = Path("/Volumes/backups")
FLOOR = 10 * 1024**3
OUTPUT_CAP = 2 * 1024**2
TIME_CAP_SECONDS = 180
PATTERN = (9, 8)
K = np.array([[1536.0, 0.0, 640.0], [0.0, 1536.0, 512.0], [0.0, 0.0, 1.0]])
OBJECT_CORNERS = np.array([(column - 4.0, row - 3.5, 0.0)
                           for row in range(8) for column in range(9)], dtype=np.float64)
SB_FLAGS = cv2.CALIB_CB_EXHAUSTIVE | cv2.CALIB_CB_ACCURACY
MIN_HULL_FRACTION = 0.005
RMS_FLAG_PX = 3.0
P95_FLAG_PX = 5.0


def digest(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def free_space():
    values = {"internal": shutil.disk_usage(ROOT).free,
              "external": shutil.disk_usage(EXTERNAL).free}
    if min(values.values()) < FLOOR:
        raise RuntimeError("both disks must retain at least 10 GiB free")
    return values


def preflight():
    """Hash the sealed TRAIN inputs; perform no image decode or pose fitting."""
    if (MANIFEST.is_symlink() or not MANIFEST.is_file() or digest(MANIFEST) != MANIFEST_SHA256 or
            PROFILE.is_symlink() or not PROFILE.is_file() or digest(PROFILE) != PROFILE_SHA256):
        raise ValueError("sealed input list or capture profile changed")
    records = json.loads(MANIFEST.read_text())
    profile = json.loads(PROFILE.read_text())
    slots = validate_profile(profile)
    if not isinstance(records, list) or len(records) != 48:
        raise ValueError("expected 48 sealed TRAIN image records")
    seen = set()
    checked = []
    for record in records:
        name = record.get("name")
        path = Path(record.get("source", ""))
        if (name not in slots or name in seen or path != TRAIN_IMAGES / name or
                path.is_symlink() or not path.is_file() or
                path.stat().st_size != record.get("bytes") or digest(path) != record.get("sha256")):
            raise ValueError(f"TRAIN image path, size or hash mismatch: {name}")
        seen.add(name)
        checked.append({"name": name, "path": str(path), "sha256": record["sha256"]})
    checked.sort(key=lambda row: slots.index(row["name"]))
    return {"schema": "mustard_checkerboard_preflight_v1", "status": "ready",
            "manifest_sha256": MANIFEST_SHA256, "profile_sha256": PROFILE_SHA256,
            "images": checked, "disk_free_bytes": free_space(), "writes": False}, profile


def rotation_degrees(a, b):
    cosine = float(np.clip((np.trace(a.T @ b) - 1.0) / 2.0, -1.0, 1.0))
    return math.degrees(math.acos(cosine))


def pose_candidates(corners):
    """Return both IPPE solutions for both indistinguishable 180° corner labels."""
    points = np.asarray(corners, dtype=np.float64)
    if points.shape != (8, 9, 2) or not np.isfinite(points).all():
        raise ValueError("expected finite 8x9 corner grid")
    candidates = []
    for flip in (0, 1):
        image_points = (points if not flip else points[::-1, ::-1]).reshape(-1, 2).copy()
        count, rvecs, tvecs, _ = cv2.solvePnPGeneric(
            OBJECT_CORNERS, image_points, K, None, flags=cv2.SOLVEPNP_IPPE)
        if count != 2:
            continue
        for planar_branch, (rvec, tvec) in enumerate(zip(rvecs, tvecs)):
            rotation = cv2.Rodrigues(rvec)[0]
            translation = np.asarray(tvec, dtype=np.float64).reshape(3)
            depths = (rotation @ OBJECT_CORNERS.T + translation[:, None])[2]
            projected = cv2.projectPoints(OBJECT_CORNERS, rvec, tvec, K, None)[0].reshape(-1, 2)
            residuals = np.linalg.norm(projected - image_points, axis=1)
            if (not np.isfinite(rotation).all() or not np.isfinite(translation).all() or
                    not np.isfinite(residuals).all() or np.min(depths) <= 0):
                continue
            candidates.append({"label_flip_180": flip, "planar_branch": planar_branch,
                               "rms_px": float(np.sqrt(np.mean(residuals**2))),
                               "p95_px": float(np.percentile(residuals, 95)),
                               "rotation": rotation, "translation": translation,
                               "center": -rotation.T @ translation})
    return candidates


def detect_frame(gray, name, slot_index):
    """Decode is caller-owned; no frame image or overlay is persisted."""
    if gray is None or gray.shape != (1024, 1280) or gray.dtype != np.uint8:
        raise ValueError("TRAIN JPEG must decode as 1280x1024 grayscale")
    found, corners = cv2.findChessboardCornersSB(gray, PATTERN, flags=SB_FLAGS)
    base = {"name": name, "slot_index": slot_index, "detected": bool(found)}
    if not found:
        return {**base, "reason": "full_board_not_detected", "candidates": []}
    points = np.asarray(corners, dtype=np.float64).reshape(8, 9, 2)
    flat = points.reshape(-1, 2)
    if (not np.isfinite(flat).all() or np.min(flat) < 0 or
            np.max(flat[:, 0]) >= 1280 or np.max(flat[:, 1]) >= 1024):
        return {**base, "reason": "invalid_or_out_of_frame_corners", "candidates": []}
    hull_fraction = float(cv2.contourArea(cv2.convexHull(flat.astype(np.float32))) / gray.size)
    bbox = list(map(int, cv2.boundingRect(flat.astype(np.float32))))
    if hull_fraction < MIN_HULL_FRACTION:
        return {**base, "reason": "board_coverage_too_small", "hull_fraction": hull_fraction,
                "bbox_xywh": bbox, "candidates": []}
    candidates = pose_candidates(points)
    return {**base, "hull_fraction": hull_fraction, "bbox_xywh": bbox,
            "reason": None if candidates else "no_positive_depth_ippe_pose", "candidates": candidates}


def transition(a, b, gap, camera_range):
    angle = rotation_degrees(a["rotation"].T, b["rotation"].T)
    step = float(np.linalg.norm(a["center"] - b["center"]))
    return (angle / (30.0 * gap))**2 + (step / (0.5 * camera_range * gap))**2


def select_cycle(frames, slot_count):
    """Select a cyclic smooth path; first detector label fixes only a gauge."""
    frames = sorted((frame for frame in frames if frame["candidates"]),
                    key=lambda frame: frame["slot_index"])
    if len(frames) < 3:
        return None
    ranges = [float(np.linalg.norm(candidate["center"])) for frame in frames
              for candidate in frame["candidates"]]
    camera_range = float(np.median(ranges))
    if camera_range <= 1e-9:
        return None
    best = None
    first = frames[0]
    for start_index, start in enumerate(first["candidates"]):
        if start["label_flip_180"] != 0:
            continue
        paths = {start_index: (start["rms_px"]**2 / 9.0, [start_index])}
        for previous, current in zip(frames, frames[1:]):
            gap = current["slot_index"] - previous["slot_index"]
            updated = {}
            for index, candidate in enumerate(current["candidates"]):
                options = [(score + transition(previous["candidates"][prior], candidate,
                                               gap, camera_range) + candidate["rms_px"]**2 / 9.0,
                            path + [index]) for prior, (score, path) in paths.items()]
                if options:
                    updated[index] = min(options, key=lambda item: (item[0], item[1]))
            paths = updated
        wrap = slot_count + first["slot_index"] - frames[-1]["slot_index"]
        for last_index, (score, path) in paths.items():
            total = score + transition(frames[-1]["candidates"][last_index], start,
                                       wrap, camera_range)
            proposal = (total, path)
            if best is None or proposal < best:
                best = proposal
    if best is None:
        return None
    chosen = [frame["candidates"][index] for frame, index in zip(frames, best[1])]
    return {"frames": frames, "chosen": chosen, "cost": float(best[0]),
            "median_camera_range_square_units": camera_range,
            "gauge": "first detector labeling fixed arbitrarily; global 180-degree board-frame reversal unresolved"}


def evaluate(prepared, profile):
    slots = validate_profile(profile)
    cv2.setNumThreads(2)
    cv2.ocl.setUseOpenCL(False)
    started = time.monotonic()
    frames = []
    for record in prepared["images"]:
        if time.monotonic() - started > TIME_CAP_SECONDS:
            raise TimeoutError("checkerboard diagnostic exceeded 180 seconds")
        gray = cv2.imread(record["path"], cv2.IMREAD_GRAYSCALE)
        frames.append(detect_frame(gray, record["name"], slots.index(record["name"])))
    selection = select_cycle(frames, len(slots))
    by_name = {frame["name"]: (frame, candidate) for frame, candidate in
               zip(selection["frames"], selection["chosen"])} if selection else {}
    views = []
    orbit_rows = []
    for record, frame in zip(prepared["images"], frames):
        chosen = by_name.get(record["name"])
        row = {"name": record["name"], "source_sha256": record["sha256"],
               "detected": frame["detected"], "reason": frame["reason"],
               "hull_fraction": frame.get("hull_fraction"), "bbox_xywh": frame.get("bbox_xywh")}
        if chosen:
            candidate = chosen[1]
            row.update({"label_flip_180": candidate["label_flip_180"],
                        "planar_branch": candidate["planar_branch"],
                        "reprojection_rms_px": candidate["rms_px"],
                        "reprojection_p95_px": candidate["p95_px"],
                        "quality_flags": (["rms_above_3px"] if candidate["rms_px"] > RMS_FLAG_PX else []) +
                                         (["p95_above_5px"] if candidate["p95_px"] > P95_FLAG_PX else []),
                        "camera_from_board_rotation": candidate["rotation"].tolist(),
                        "camera_from_board_translation_square_units": candidate["translation"].tolist(),
                        "camera_center_board_square_units": candidate["center"].tolist(),
                        "candidate_reprojection_rms_px": [round(item["rms_px"], 6)
                                                           for item in chosen[0]["candidates"]]})
            orbit_rows.append({"name": record["name"], "center": candidate["center"],
                               "camera_to_world_rotation": candidate["rotation"].T})
        views.append(row)
    orbit = orbit_evaluate(orbit_rows, profile) if len(orbit_rows) >= 12 else None
    if orbit:
        orbit = {key: orbit.get(key) for key in
                 ("status", "reason", "failures", "registered", "coverage", "planar_axis_ratio",
                  "winding_degrees", "reversed_significant_step_fraction",
                  "inward_facing_fraction", "maximum_adjacent_center_step_over_radius",
                  "maximum_adjacent_orientation_step_degrees")}
    return {"schema": "mustard_checkerboard_assisted_pose_diagnostic_v1",
            "status": "diagnostic_only" if selection else "unavailable",
            "scope": "48 sealed TRAIN photos; image-derived board corners only",
            "assumed_intrinsics": {"fx": 1536.0, "fy": 1536.0, "cx": 640.0, "cy": 512.0,
                                   "distortion": "zero assumed, not calibrated"},
            "board": {"inner_corners_columns_rows": [9, 8], "square_units": "arbitrary",
                      "frame": "centered inner-corner grid, z=0"},
            "manifest_sha256": MANIFEST_SHA256, "profile_sha256": PROFILE_SHA256,
            "software_sha256": digest(Path(__file__)), "opencv_version": cv2.__version__,
            "detected_count": sum(bool(frame["candidates"]) for frame in frames),
            "selection": ({"cost": selection["cost"],
                           "median_camera_range_square_units": selection["median_camera_range_square_units"],
                           "gauge": selection["gauge"]} if selection else None),
            "orbit_diagnostic": orbit, "views": views,
            "limitations": "Board pose is relative to moving board, not a board-free object pose or metric truth; global 180-degree labeling and intrinsic assumptions remain"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evaluate", action="store_true")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--worker", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    prepared, profile = preflight()
    if not args.evaluate:
        print(json.dumps({key: prepared[key] for key in
                          ("schema", "status", "manifest_sha256", "profile_sha256", "disk_free_bytes", "writes")}
                         | {"image_count": len(prepared["images"])}, sort_keys=True))
        return
    if args.output is None or args.output.exists() or args.output.is_symlink():
        raise ValueError("evaluation requires a fresh external output file")
    if (args.output.parent.is_symlink() or not args.output.parent.is_dir() or
            args.output.parent.stat().st_dev != EXTERNAL.stat().st_dev):
        raise ValueError("output parent must be an existing external directory")
    if not args.worker:
        command = [sys.executable, "-m", "scripts.object_motion.checkerboard_pose",
                   "--evaluate", "--worker", "--output", str(args.output)]
        environment = os.environ.copy()
        environment["OPENCV_OPENCL_RUNTIME"] = "disabled"
        environment["TMPDIR"] = str(args.output.parent)
        environment["OMP_NUM_THREADS"] = "2"
        child = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                 env=environment, cwd=ROOT)
        deadline = time.monotonic() + TIME_CAP_SECONDS
        try:
            while child.poll() is None:
                if time.monotonic() > deadline:
                    raise TimeoutError("checkerboard worker exceeded 180 seconds")
                free_space()
                time.sleep(1)
            stdout, stderr = child.communicate(timeout=5)
        except BaseException:
            child.kill()
            child.wait()
            raise
        if child.returncode or len(stdout) > 1024 or len(stderr) > 4096:
            raise RuntimeError(f"checkerboard worker failed ({child.returncode}): {stderr[:1024]!r}")
        print(stdout.decode().strip())
        return
    result = evaluate(prepared, profile)
    if digest(MANIFEST) != MANIFEST_SHA256 or digest(PROFILE) != PROFILE_SHA256:
        raise ValueError("input manifest or profile changed during evaluation")
    for record in prepared["images"]:
        if digest(record["path"]) != record["sha256"]:
            raise ValueError("TRAIN JPEG changed during evaluation")
    free_space()
    payload = (json.dumps(result, sort_keys=True, indent=2) + "\n").encode()
    if len(payload) > OUTPUT_CAP:
        raise ValueError("diagnostic exceeds 2 MiB output cap")
    with args.output.open("xb") as stream:
        stream.write(payload)
    print(json.dumps({"status": result["status"], "detected_count": result["detected_count"],
                      "report": str(args.output), "report_sha256": digest(args.output)}, sort_keys=True))


if __name__ == "__main__":
    main()
