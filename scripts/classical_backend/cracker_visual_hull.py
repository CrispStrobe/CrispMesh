"""Bounded image-only visual-hull producer for sealed 60-view cracker-box.

No scanner mesh, reference transform, Berkeley pose/calibration, depth map or
other reconstruction is read. A separate score may inspect the sealed hull.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import signal
import struct
import sys
import time

import numpy as np
from PIL import Image, ImageDraw

from scripts.classical_backend.run import stage
from scripts.classical_backend.visual_hull_core import (
    PinholeView, boundary_touched, carve_chunk, exposed_cube_mesh,
    sparse_cube, voxel_centers,
)


ROOT = Path(__file__).resolve().parents[2]
DATA = Path("/Volumes/backups/code/crispmesh-data")
SOURCE = DATA / "turntable-fresh-openmvs-005"
OUTPUT = DATA / "cracker-visual-hull-001"
RESULT_SHA256 = "dafc6ccb2a90f0c5a45748291571455237acd8ae08da01dd80adc10222a3625e"
MASK_REPORT_SHA256 = "dc5ff13e84f66b810709a85aef41b6459abb6b9f78f99996e632862572923fef"
MODEL_SHA256 = {
    "cameras.bin": "c746ffa33ade1b725399771b99915904a53d2a134c5738fc94a1ebaf70d84ee4",
    "images.bin": "cb6a663a0d5ffae69a5f1b2335408ea00b447c8aa83ff7028759843e61a45ae8",
    "points3D.bin": "8b8dd2d156ee517b8dfb8a2dab00c7925e6ba18cdf06e4db3f7b5d5f14f18693",
}
NAMES = {f"NP3_{angle:03d}.jpg" for angle in range(0, 360, 6)}
GRID = 160
CHUNK = 65_536
MIN_VIEWS = 48
MIN_SPARSE_POINTS = 1_000
MAX_TRIANGLES = 500_000
MAX_OUTPUT = 256 * 1024**2
MAX_MESH = 128 * 1024**2
MAX_PREVIEW = 16 * 1024**2
MAX_LOG = 16 * 1024**2
MAX_RSS = 2 * 1024**3
MIN_FREE = 10 * 1024**3
WORKER_SECONDS = 300
SUPERVISOR_SECONDS = 330


def digest(path: Path) -> str:
    result = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(block)
    return result.hexdigest()


def disk_floor(*, allowance: int = 0) -> dict[str, int]:
    free = {"internal": shutil.disk_usage(ROOT).free, "external": shutil.disk_usage(DATA).free}
    if free["internal"] < MIN_FREE or free["external"] < MIN_FREE + allowance:
        raise ValueError("both disks must retain at least 10 GiB and output allowance")
    return free


def _sealed_json(path: Path, expected: str) -> dict:
    if path.is_symlink() or not path.is_file() or path.stat().st_size > 2 * 1024**2 or digest(path) != expected:
        raise ValueError(f"sealed JSON differs: {path.name}")
    value = json.loads(path.read_text())
    if not isinstance(value, dict):
        raise ValueError("sealed JSON is not an object")
    return value


def validate_mask_report(report: dict) -> dict[str, dict]:
    entries = report.get("images")
    if (report.get("schema") != "classical_dense_masks_v1" or report.get("status") != "complete" or
            report.get("semantics") != "photo-derived coarse pose support, not a silhouette or ground truth" or
            report.get("ignore_mask_label") != 0 or
            report.get("undistorted_model_sha256") != MODEL_SHA256 or
            not isinstance(entries, list) or len(entries) != 60 or
            any(not isinstance(row, dict) for row in entries)):
        raise ValueError("incomplete or malformed sealed 60-view mask report")
    listed = {row.get("name"): row for row in entries}
    if len(listed) != 60 or set(listed) != NAMES:
        raise ValueError("incomplete or malformed sealed 60-view mask report")
    for name, row in listed.items():
        if (row.get("native_mask_name") != name.replace(".jpg", ".mask.png") or
                any(not isinstance(row.get(key), str) or len(row[key]) != 64 or
                    any(char not in "0123456789abcdef" for char in row[key])
                    for key in ("mask_sha256", "undistorted_image_sha256"))):
            raise ValueError("incomplete or malformed sealed 60-view mask report")
    return listed


def load_sealed_views():
    """Read only sealed 005 camera, image, mask and sparse-point inputs."""
    if SOURCE.is_symlink() or not SOURCE.is_dir():
        raise ValueError("sealed producer root missing or linked")
    result_path, report_path = SOURCE / "result.json", SOURCE / "masks" / "report.json"
    result = _sealed_json(result_path, RESULT_SHA256)
    report = _sealed_json(report_path, MASK_REPORT_SHA256)
    if (result.get("schema") != "classical_fresh_masked_complete_v1" or result.get("status") != "complete" or
            result.get("sources_unchanged") is not True):
        raise ValueError("sealed producer or mask semantics differ")
    listed = validate_mask_report(report)
    model_dir = SOURCE / "dense" / "sparse"
    if model_dir.is_symlink() or not model_dir.is_dir():
        raise ValueError("undistorted sparse model directory differs")
    for name, expected in MODEL_SHA256.items():
        path = model_dir / name
        if path.is_symlink() or not path.is_file() or digest(path) != expected:
            raise ValueError("undistorted sparse model differs")
    import pycolmap
    model = pycolmap.Reconstruction(str(model_dir))
    registered = {image.name: image for image in model.images.values() if image.has_pose}
    if len(registered) != 60 or set(registered) != NAMES:
        raise ValueError("exact 60 TRAIN name inventory differs")
    photos_dir, masks_dir = SOURCE / "dense" / "images", SOURCE / "masks"
    if (photos_dir.is_symlink() or masks_dir.is_symlink() or
            {path.name for path in photos_dir.iterdir()} != NAMES or
            {path.name for path in masks_dir.iterdir()} !=
            {"report.json"} | {row["native_mask_name"] for row in listed.values()}):
        raise ValueError("exact undistorted image/mask inventory differs")
    views = []
    source_hashes = {}
    for name in sorted(NAMES):
        row, image = listed[name], registered[name]
        camera = model.cameras[image.camera_id]
        if (camera.model.name != "PINHOLE" or len(camera.params) != 4 or
                row.get("native_mask_name") != name.replace(".jpg", ".mask.png") or
                not isinstance(row.get("mask_sha256"), str) or
                not isinstance(row.get("undistorted_image_sha256"), str)):
            raise ValueError("undistorted camera or source row differs")
        photo, mask_path = photos_dir / name, masks_dir / row["native_mask_name"]
        if (photo.is_symlink() or mask_path.is_symlink() or
                not photo.is_file() or not mask_path.is_file() or
                digest(photo) != row["undistorted_image_sha256"] or
                digest(mask_path) != row["mask_sha256"]):
            raise ValueError("undistorted image/mask hash differs")
        with Image.open(photo) as rgb, Image.open(mask_path) as mask_file:
            if (rgb.format != "JPEG" or rgb.mode != "RGB" or
                    rgb.size != (camera.width, camera.height) or
                    mask_file.format != "PNG" or mask_file.mode != "L" or
                    mask_file.size != rgb.size):
                raise ValueError("undistorted image/mask format differs")
            pixels = np.asarray(mask_file).copy()
        views.append(PinholeView(image.cam_from_world.rotation.matrix(),
                                 np.asarray(image.cam_from_world.translation),
                                 tuple(camera.params), pixels))
        source_hashes[name] = {"image_sha256": row["undistorted_image_sha256"],
                               "mask_sha256": row["mask_sha256"],
                               "mask_name": row["native_mask_name"]}
    xyz = np.asarray([point.xyz for point in model.points3D.values()], dtype=np.float64)
    if len(xyz) < MIN_SPARSE_POINTS or not np.isfinite(xyz).all():
        raise ValueError("insufficient finite image-only sparse points")
    return tuple(views), xyz, source_hashes


def recheck_sealed(source_hashes: dict) -> None:
    if (digest(SOURCE / "result.json") != RESULT_SHA256 or
            digest(SOURCE / "masks" / "report.json") != MASK_REPORT_SHA256):
        raise ValueError("sealed reports changed during hull generation")
    for name, expected in MODEL_SHA256.items():
        if digest(SOURCE / "dense" / "sparse" / name) != expected:
            raise ValueError("undistorted sparse model changed")
    for name, row in source_hashes.items():
        if (digest(SOURCE / "dense" / "images" / name) != row["image_sha256"] or
                digest(SOURCE / "masks" / row["mask_name"]) != row["mask_sha256"]):
            raise ValueError("undistorted image/mask changed")


def write_ply(path: Path, vertices: np.ndarray, faces: np.ndarray) -> None:
    """Fresh binary little-endian XYZ/triangle PLY; no colors or cleanup."""
    vertices = np.asarray(vertices, dtype=np.float64)
    faces = np.asarray(faces, dtype=np.int32)
    if (vertices.ndim != 2 or vertices.shape[1] != 3 or not np.isfinite(vertices).all() or
            faces.ndim != 2 or faces.shape[1] != 3 or len(faces) > MAX_TRIANGLES or
            np.any(faces < 0) or np.any(faces >= len(vertices))):
        raise ValueError("invalid neutral triangle mesh")
    header = ("ply\nformat binary_little_endian 1.0\n"
              f"element vertex {len(vertices)}\nproperty double x\nproperty double y\nproperty double z\n"
              f"element face {len(faces)}\nproperty list uchar int vertex_indices\nend_header\n").encode()
    if len(header) + len(vertices) * 24 + len(faces) * 13 > MAX_MESH:
        raise ValueError("neutral mesh exceeds 128 MiB cap")
    with Path(path).open("xb") as stream:
        stream.write(header)
        stream.write(vertices.astype("<f8", copy=False).tobytes())
        for a, b, c in faces:
            stream.write(struct.pack("<Biii", 3, int(a), int(b), int(c)))


def write_neutral_preview(path: Path, vertices: np.ndarray, faces: np.ndarray,
                          center: np.ndarray, side: float) -> None:
    """Candidate-only fixed-orthographic XY/XZ/YZ area sample preview."""
    points = vertices[faces]
    cross = np.cross(points[:, 1] - points[:, 0], points[:, 2] - points[:, 0])
    area = np.linalg.norm(cross, axis=1) / 2
    if not np.isfinite(area).all() or area.sum() <= 0:
        raise ValueError("neutral preview has no positive-area faces")
    rng = np.random.default_rng(2030)
    selection = rng.choice(len(faces), 10_000, replace=True, p=area / area.sum())
    bary = rng.random((10_000, 2))
    flip = bary.sum(axis=1) > 1
    bary[flip] = 1 - bary[flip]
    sample = points[selection, 0] + bary[:, :1] * (points[selection, 1] - points[selection, 0]) + \
        bary[:, 1:] * (points[selection, 2] - points[selection, 0])
    canvas = Image.new("RGB", (768, 256), "white")
    draw = ImageDraw.Draw(canvas)
    for panel, (first, second) in enumerate(((0, 1), (0, 2), (1, 2))):
        x = np.clip(((sample[:, first] - center[first]) / side + 0.5) * 224 + panel * 256 + 16, 0, 767)
        y = np.clip((0.5 - (sample[:, second] - center[second]) / side) * 224 + 16, 0, 255)
        for px, py in zip(x.astype(int), y.astype(int)):
            draw.point((int(px), int(py)), fill=(25, 25, 25))
    with Path(path).open("xb") as stream:
        canvas.save(stream, format="PNG")
    if Path(path).stat().st_size > MAX_PREVIEW:
        raise ValueError("neutral preview exceeds 16 MiB cap")


def _timeout(_signum, _frame):
    raise TimeoutError("visual-hull worker exceeded 300 seconds")


def abstention_state(occupied: np.ndarray) -> dict:
    """Preserve the exact empty/truncation gate outcome in the receipt."""
    count = int(occupied.sum())
    touched = bool(boundary_touched(occupied))
    return {"occupied_voxels": count, "boundary_touched": touched,
            "abstention_reason": "empty" if count == 0 else "outer_grid_touch" if touched else None}


def write_report(path: Path, report: dict) -> None:
    payload = (json.dumps(report, sort_keys=True, allow_nan=False, indent=2) + "\n").encode()
    if len(payload) > 2 * 1024**2:
        raise ValueError("hull report exceeds 2 MiB cap")
    with path.open("xb") as stream:
        stream.write(payload)


def worker(output: Path) -> dict:
    if hasattr(signal, "SIGALRM"):
        signal.signal(signal.SIGALRM, _timeout)
        signal.alarm(WORKER_SECONDS)
    try:
        disk_floor()
        views, sparse_xyz, source_hashes = load_sealed_views()
        center, side = sparse_cube(sparse_xyz)
        occupied = np.zeros((GRID, GRID, GRID), dtype=bool)
        view_counts = np.zeros(61, dtype=np.int64)
        for start in range(0, GRID**3, CHUNK):
            stop = min(start + CHUNK, GRID**3)
            xyz = voxel_centers(start, stop, GRID, center, side)
            accepted, visible = carve_chunk(xyz, views, min_visible=MIN_VIEWS)
            occupied.ravel()[start:stop] = accepted
            view_counts += np.bincount(visible, minlength=61)
            if shutil.disk_usage(DATA).free < MIN_FREE or shutil.disk_usage(ROOT).free < MIN_FREE:
                raise ValueError("10 GiB disk reserve reached")
        state = abstention_state(occupied)
        recheck_sealed(source_hashes)
        disk_floor()
        report = {"schema": "cracker_visual_hull_v1",
                  "status": "abstained" if state["abstention_reason"] else "complete",
                  "semantics": "coarse photo-derived mask support, not certified object silhouette",
                  "source_sha256": {"producer_result": RESULT_SHA256, "mask_report": MASK_REPORT_SHA256,
                                    "undistorted_model": MODEL_SHA256,
                                    "image_mask_manifest": hashlib.sha256(json.dumps(source_hashes, sort_keys=True).encode()).hexdigest()},
                  "runner_sha256": digest(Path(__file__)),
                  "core_sha256": digest(Path(sys.modules["scripts.classical_backend.visual_hull_core"].__file__)),
                  "grid": {"side_cells": GRID, "chunk": CHUNK, "minimum_visible_views": MIN_VIEWS,
                           "sparse_bounds_quantiles": [0.01, 0.99], "cube_side_multiplier": 1.5,
                           "center": center.tolist(), "side": side},
                  "sparse_points": len(sparse_xyz), **state,
                  "voxel_volume_world_units": float((side / GRID) ** 3),
                  "hull_volume_world_units": float(occupied.sum() * (side / GRID) ** 3),
                  "visible_view_histogram_all_grid_cells": view_counts.tolist(),
                  "reference_used": False, "camera_reference_used": False,
                  "depth_used": False, "dense_cloud_used": False,
                  "interpretation": "image-only support envelope; scanner score requires separate authorization"}
        if state["abstention_reason"]:
            report.update(neutral_mesh=None, neutral_preview=None)
            write_report(output / "hull-report.json", report)
            return report
        vertices, faces = exposed_cube_mesh(occupied, center, side, max_triangles=MAX_TRIANGLES)
        mesh_path, preview_path = output / "hull.ply", output / "neutral-preview.png"
        write_ply(mesh_path, vertices, faces)
        write_neutral_preview(preview_path, vertices, faces, center, side)
        recheck_sealed(source_hashes)
        disk_floor()
        report.update(neutral_mesh={"vertices": len(vertices), "triangles": len(faces),
                                    "sha256": digest(mesh_path), "bytes": mesh_path.stat().st_size},
                      neutral_preview={"sha256": digest(preview_path), "bytes": preview_path.stat().st_size,
                                       "views": ["XY", "XZ", "YZ"], "area_samples": 10000, "seed": 2030})
        write_report(output / "hull-report.json", report)
        return report
    finally:
        if hasattr(signal, "SIGALRM"):
            signal.alarm(0)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--worker", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    output = args.output
    if output != OUTPUT:
        raise ValueError("only the frozen fresh external output path is allowed")
    if args.worker:
        worker(output)
        return
    if (output.exists() or output.is_symlink() or output.parent != DATA or
            DATA.is_symlink() or not DATA.is_dir()):
        raise ValueError("visual-hull output must be fresh on external volume")
    disk_floor(allowance=MAX_OUTPUT)
    # Duplicate the worker's full read-only validation before creating output.
    # The worker revalidates because sources may change between processes.
    load_sealed_views()
    disk_floor(allowance=MAX_OUTPUT)
    output.mkdir()
    started = time.monotonic()
    status = {"schema": "cracker_visual_hull_supervisor_v1", "status": "running",
              "source_result_sha256": RESULT_SHA256, "mask_report_sha256": MASK_REPORT_SHA256,
              "runner_sha256": digest(Path(__file__))}
    try:
        executed = stage(output, "carve", [sys.executable, "-m", "scripts.classical_backend.cracker_visual_hull",
                                             "--worker", "--output", str(output)],
                         started + SUPERVISOR_SECONDS, MAX_OUTPUT, MAX_LOG, MAX_RSS,
                         extra_reserve_paths=(ROOT,))
        report_path = output / "hull-report.json"
        report = json.loads(report_path.read_text())
        if report.get("status") not in {"complete", "abstained"} or report.get("runner_sha256") != status["runner_sha256"]:
            raise ValueError("worker did not seal exact hull report")
        status.update(status=report["status"], stage=executed, hull_report_sha256=digest(report_path),
                      occupied_voxels=report["occupied_voxels"],
                      boundary_touched=report["boundary_touched"],
                      abstention_reason=report["abstention_reason"])
    except Exception as error:
        status.update(status="failed", failure=f"{type(error).__name__}: {error}")
        raise
    finally:
        status["seconds"] = round(time.monotonic() - started, 3)
        with (output / "result.json").open("x") as stream:
            json.dump(status, stream, sort_keys=True, indent=2)
            stream.write("\n")
        disk_floor()


if __name__ == "__main__":
    main()
