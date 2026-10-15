"""Read-only TRAIN-only independent essential-pose audit for two mustard databases."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
from pathlib import Path
import shutil
import signal
import sqlite3
import time

import cv2
import numpy as np


ROOT = Path(__file__).resolve().parents[2]
DATA = Path("/Volumes/backups/code/crispmesh-data")
SOURCES = {
    "cached": (DATA / "mustard-sfm-masked-fixed-exhaustive-001",
               "5858be16dfa3f0fc50303d0eb4a001e21ee14036f67755a9997d5dac374b3075",
               "5a83c3e05e2b66887411799ecc8c5f3f41e3b8af3bbd775d308055ef7ee0041e"),
    "sam": (DATA / "mustard-feature-mask-pair-001-sam",
            "09068fa4999a29fd9f61549ef4c56f3a92e7dbfc5cb2be62bf51ea9b72aee7d8",
            "4d6ce1bba0534c25035168d8c82565cf99ad2415c5c7327fb7ca6f0e6c4401d3"),
}
PANEL = (
    ("near", "NP3_006.jpg", "NP3_012.jpg", 1, (136, 136)),
    ("near", "NP3_012.jpg", "NP3_018.jpg", 1, (129, 129)),
    ("middle", "NP3_012.jpg", "NP3_336.jpg", 5, (41, 42)),
    ("middle", "NP3_036.jpg", "NP3_126.jpg", 12, (24, 24)),
    ("opposing", "NP3_030.jpg", "NP3_198.jpg", 23, (25, 26)),
    ("opposing", "NP3_036.jpg", "NP3_222.jpg", 23, (25, 25)),
)
SEEDS = (17, 23, 31)
MAX_IMAGE_ID = 2147483647
THRESHOLD_NORM = 2 / 1536
MIN_FREE = 10 * 1024**3
MAX_REPORT = 20 * 1024**2
MAX_SECONDS = 30


def sha(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def disk_floor() -> dict[str, int]:
    values = {str(ROOT): shutil.disk_usage(ROOT).free,
              str(DATA): shutil.disk_usage(DATA).free}
    if min(values.values()) < MIN_FREE:
        raise OSError("both disks require at least 10 GiB free")
    return values


def safe_database(run: Path, expected_db: str, expected_result: str):
    if run.is_symlink() or not run.is_dir():
        raise ValueError("missing or linked source run")
    db, producer = run / "database.db", run / "result.json"
    if (db.is_symlink() or producer.is_symlink() or not db.is_file() or
            not producer.is_file() or not 0 < db.stat().st_size < 20 << 20 or
            sha(db) != expected_db or sha(producer) != expected_result):
        raise ValueError("sealed source database or producer differs")
    sidecars = {}
    for suffix in ("-wal", "-shm"):
        path = Path(str(db) + suffix)
        if path.is_symlink() or (path.exists() and (not path.is_file() or path.stat().st_size)):
            raise ValueError("nonempty or linked SQLite sidecar")
        sidecars[suffix] = bool(path.exists())
    return db, producer, sidecars


def read_panel(db: Path, arm: str):
    connection = sqlite3.connect(db.resolve().as_uri() + "?mode=ro&immutable=1", uri=True)
    try:
        connection.execute("PRAGMA query_only=ON")
        names = dict(connection.execute("SELECT image_id,name FROM images"))
        if len(names) != 48 or len(set(names.values())) != 48:
            raise ValueError("expected 48 distinct TRAIN image names")
        ids = {name: image_id for image_id, name in names.items()}
        camera = connection.execute("SELECT model,width,height,params FROM cameras").fetchall()
        if len(camera) != 1 or camera[0][:3] != (2, 1280, 1024) or \
                camera[0][3] is None or len(camera[0][3]) != 32 or \
                not np.array_equal(np.frombuffer(camera[0][3], dtype="<f8"), [1536, 640, 512, 0]):
            raise ValueError("camera is not fixed initial SIMPLE_RADIAL [1536,640,512,0]")
        rows = []
        for stratum, left, right, separation, expected in PANEL:
            if left not in ids or right not in ids:
                raise ValueError("panel name missing from TRAIN inventory")
            first, second = sorted((ids[left], ids[right]))
            pair_id = first * MAX_IMAGE_ID + second
            record = connection.execute(
                "SELECT rows,cols,data,config FROM two_view_geometries WHERE pair_id=?",
                (pair_id,)).fetchone()
            required_count = expected[0 if arm == "cached" else 1]
            if (record is None or record[0] != required_count or record[1] != 2 or
                    record[2] is None or len(record[2]) != required_count * 8 or
                    required_count < 20):
                raise ValueError("frozen panel verified-match row changed")
            matches = np.frombuffer(record[2], dtype="<u4").reshape(required_count, 2).copy()
            if len(np.unique(matches, axis=0)) != required_count:
                raise ValueError("duplicate verified feature-index pair")
            points = {}
            for image_id in (first, second):
                key = connection.execute("SELECT rows,cols,data FROM keypoints WHERE image_id=?",
                                         (image_id,)).fetchone()
                if (key is None or not 0 < key[0] <= 10000 or key[1] != 6 or
                        key[2] is None or len(key[2]) != key[0] * key[1] * 4):
                    raise ValueError("invalid keypoint blob")
                points[image_id] = np.frombuffer(key[2], dtype="<f4").reshape(key[0], 6)[:, :2].astype(float)
            if (np.any(matches[:, 0] >= len(points[first])) or
                    np.any(matches[:, 1] >= len(points[second]))):
                raise ValueError("verified match index outside keypoint array")
            x_first = points[first][matches[:, 0]]
            x_second = points[second][matches[:, 1]]
            x_left, x_right = ((x_first, x_second) if ids[left] == first else
                               (x_second, x_first))
            if (not np.isfinite(x_left).all() or not np.isfinite(x_right).all() or
                    len(np.unique(x_left, axis=0)) < 16 or
                    len(np.unique(x_right, axis=0)) < 16):
                raise ValueError("insufficient unique finite measured keypoints")
            rows.append({"stratum": stratum, "left": left, "right": right,
                         "cyclic_separation": separation,
                         "verified_correspondences": required_count,
                         "two_view_config": int(record[3]),
                         "points_left": (x_left - [640, 512]) / 1536,
                         "points_right": (x_right - [640, 512]) / 1536})
        return rows, sorted(names.values())
    finally:
        connection.close()


def rotation_difference_degrees(left, right):
    cosine = float(np.clip((np.trace(left @ right.T) - 1) / 2, -1, 1))
    return math.degrees(math.acos(cosine))


def ray_angles_degrees(points_left, points_right, rotation, positive):
    one = np.column_stack((points_left[positive], np.ones(np.count_nonzero(positive))))
    two = np.column_stack((points_right[positive], np.ones(np.count_nonzero(positive)))) @ rotation
    one /= np.linalg.norm(one, axis=1)[:, None]
    two /= np.linalg.norm(two, axis=1)[:, None]
    return np.degrees(np.arccos(np.clip(np.einsum("ij,ij->i", one, two), -1, 1)))


def fit_pair(row: dict) -> dict:
    left, right = row["points_left"], row["points_right"]
    output = {key: row[key] for key in ("stratum", "left", "right", "cyclic_separation",
                                       "verified_correspondences", "two_view_config")}
    fits = []
    for seed in SEEDS:
        cv2.setRNGSeed(seed)
        try:
            essential, inliers = cv2.findEssentialMat(
                left, right, np.eye(3), method=cv2.RANSAC, prob=.999,
                threshold=THRESHOLD_NORM, maxIters=2000)
            if essential is None or inliers is None or essential.shape != (3, 3):
                fits.append({"seed": seed, "status": "unavailable", "reason": "no unique 3x3 essential matrix"})
                continue
            e_mask = inliers.ravel().astype(bool)
            count = int(e_mask.sum())
            if count < 5:
                fits.append({"seed": seed, "status": "unavailable", "reason": "fewer than five E inliers",
                             "essential_inliers": count})
                continue
            cheirality, rotation, translation, pose_mask = cv2.recoverPose(
                essential, left, right, np.eye(3), mask=e_mask.astype("uint8")[:, None])
            positive = pose_mask.ravel().astype(bool)
            angle = rotation_difference_degrees(rotation, np.eye(3))
            ray_angles = ray_angles_degrees(left, right, rotation, positive)
            median_parallax = float(np.median(ray_angles)) if len(ray_angles) else None
            reliable = (count >= 15 and count / len(left) >= .65 and
                        cheirality >= 12 and cheirality / count >= .6 and
                        median_parallax is not None and median_parallax >= 1)
            fits.append({"seed": seed, "status": "passed" if reliable else "unavailable",
                         "essential_inliers": count, "essential_fraction": count / len(left),
                         "cheirality_positive": int(cheirality),
                         "cheirality_fraction_of_essential": int(cheirality) / count,
                         "rotation_degrees": angle, "rotation": rotation.tolist(),
                         "translation_unit": translation.ravel().tolist(),
                         "median_inter_ray_parallax_degrees": median_parallax,
                         "reason": None if reliable else "essential/cheirality/parallax gate failed"})
        except cv2.error as error:
            fits.append({"seed": seed, "status": "unavailable", "reason": str(error)[:300]})
    cv2.setRNGSeed(SEEDS[0])
    try:
        homography, h_mask = cv2.findHomography(
            left, right, method=cv2.RANSAC, ransacReprojThreshold=THRESHOLD_NORM,
            maxIters=2000, confidence=.999)
        h_count = int(h_mask.sum()) if homography is not None and h_mask is not None else 0
    except cv2.error:
        h_count = 0
    output["seeds"] = fits
    output["homography_inliers"] = h_count
    passed = [fit for fit in fits if fit["status"] == "passed"]
    if len(passed) != len(SEEDS):
        output.update({"status": "unavailable", "reason": "one or more seed fits failed reliability gate"})
        return output
    rotations = [np.asarray(fit["rotation"]) for fit in fits]
    spread = max(rotation_difference_degrees(a, b) for i, a in enumerate(rotations)
                 for b in rotations[i + 1:])
    output["max_interseed_rotation_difference_degrees"] = spread
    if spread > 5:
        output.update({"status": "unavailable", "reason": "rotation unstable across fixed seeds"})
    elif h_count >= min(fit["essential_inliers"] for fit in fits):
        output.update({"status": "unavailable", "reason": "homography matches or exceeds essential support"})
    else:
        angle = float(np.median([fit["rotation_degrees"] for fit in fits]))
        output["median_rotation_degrees"] = angle
        output["status"] = "near_identity" if angle < 5 else "distinct" if angle > 15 else "indeterminate"
    return output


def run(output: Path) -> dict:
    output = Path(output)
    if (output.exists() or output.is_symlink() or output.parent.is_symlink() or
            not output.parent.is_dir() or output.parent.resolve() != DATA):
        raise ValueError("fresh output file required directly on backup data volume")
    free_before = disk_floor()
    source, panels, inventories = {}, {}, {}
    for arm, (folder, db_hash, producer_hash) in SOURCES.items():
        db, producer, sidecars = safe_database(folder, db_hash, producer_hash)
        panel, names = read_panel(db, arm)
        source[arm] = {"database": str(db), "database_sha256": db_hash,
                       "producer_result_sha256": producer_hash, "sidecars": sidecars}
        panels[arm], inventories[arm] = panel, names
    if inventories["cached"] != inventories["sam"]:
        raise ValueError("TRAIN name inventories differ")
    started = time.monotonic()
    cv2.setNumThreads(1)
    report = {"schema": "mustard_independent_two_view_pose_v1", "status": "complete",
              "scope": "TRAIN-only verified-feature E estimation; no saved SfM poses/GT/depth/heldout",
              "source": source, "runner_sha256": sha(Path(__file__)),
              "software": {"opencv_version": cv2.__version__, "numpy_version": np.__version__},
              "protocol": {"seeds": SEEDS, "ransac_threshold_pixels": 2,
                           "ransac_confidence": .999, "max_ransac_iterations": 2000,
                           "camera_assumption": "SIMPLE_RADIAL f=1536, cx=640, cy=512, k=0",
                           "elapsed_cap_seconds": MAX_SECONDS},
              "disk_free_before_bytes": free_before, "arms": {}}
    for arm in SOURCES:
        report["arms"][arm] = [fit_pair(row) for row in panels[arm]]
    report["elapsed_seconds"] = time.monotonic() - started
    for arm, (folder, db_hash, producer_hash) in SOURCES.items():
        safe_database(folder, db_hash, producer_hash)
    if sha(Path(__file__)) != report["runner_sha256"]:
        raise ValueError("runner changed during fit")
    report["disk_free_after_bytes"] = disk_floor()
    payload = (json.dumps(report, indent=2, allow_nan=False) + "\n").encode()
    if len(payload) >= MAX_REPORT:
        raise ValueError("report exceeds 20 MiB")
    output.write_bytes(payload)
    return report


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--approve-reviewed-preflight", action="store_true")
    args = parser.parse_args()
    if not args.approve_reviewed_preflight:
        parser.error("real pose audit requires root-reviewed preflight")
    def timeout(_signum, _frame):
        raise TimeoutError("two-view pose audit exceeded 30 seconds")
    signal.signal(signal.SIGALRM, timeout)
    signal.setitimer(signal.ITIMER_REAL, MAX_SECONDS)
    try:
        result = run(args.output)
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
    print(json.dumps({"status": result["status"], "output": str(args.output),
                      "elapsed_seconds": result["elapsed_seconds"]}))


if __name__ == "__main__":
    main()
