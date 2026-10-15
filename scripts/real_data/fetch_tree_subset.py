#!/usr/bin/env python3
"""Fetch a small, revision-pinned tree photo set within explicit byte limits."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path, PurePosixPath
import shutil
import tempfile
import time
import urllib.request


REPO = Path(__file__).resolve().parents[2]
DEFAULT_ROOT = REPO / ".local-tools/test-data/tree-subset"
DATASET = "Matt1up/tree-minnetonka-photogrammetry"
REVISION = "5f9de5e4a1be429b192a928cf1359c066dadb4b3"
BASE = f"https://huggingface.co/datasets/{DATASET}/resolve/{REVISION}/"
TREE_API = f"https://huggingface.co/api/datasets/{DATASET}/tree/{REVISION}?recursive=true"
GIB = 1024**3
MAX_BATCH_BYTES = GIB
MAX_PHOTO_BYTES = 250 * 1000**2
MIN_FREE_BYTES = 10 * GIB
MAX_SINGLE_BYTES = 60 * 1000**2
MAX_FILE_SECONDS = 180
EXCLUDED = {"The_Tree-88.jpg", "The_Tree-223.jpg", "The_Tree-137.jpg", "The_Tree-235.jpg"}


def safe_path(root: Path, relative: str) -> Path:
    path = PurePosixPath(relative)
    if (path.is_absolute() or not path.parts or "\\" in relative or
            str(path) != relative or any(part in (".", "..") for part in path.parts)):
        raise ValueError(f"unsafe dataset path: {relative!r}")
    result = root.joinpath(*path.parts)
    if root.is_symlink() or any(parent.is_symlink() for parent in (root, *list(result.parents)[:-1])) or result.is_symlink():
        raise ValueError(f"symlink in dataset path: {relative!r}")
    if not result.resolve().is_relative_to(root.resolve()):
        raise ValueError(f"path escapes output root: {relative!r}")
    return result


def api_files() -> dict[str, dict]:
    with urllib.request.urlopen(TREE_API, timeout=30) as response:
        if response.headers.get("Link"):
            raise ValueError("dataset metadata is paginated; cannot use incomplete listing")
        data = response.read(2_000_001)
        if len(data) > 2_000_000:
            raise ValueError("dataset metadata exceeds 2 MB limit")
        records = json.loads(data)
    return {item["path"]: item for item in records if item.get("type") == "file"}


def expected_digest(item: dict) -> str | None:
    return item.get("lfs", {}).get("oid")


def verify_digest(path: Path, item: dict) -> bool:
    lfs_hash = expected_digest(item)
    if lfs_hash:
        return sha256_file(path) == lfs_hash
    # Small Git files expose a blob object ID, rather than an LFS SHA-256.
    object_id = item.get("oid")
    if not object_id:
        raise ValueError(f"missing source digest: {path}")
    contents = path.read_bytes()
    git_blob = hashlib.sha1(f"blob {len(contents)}\0".encode() + contents).hexdigest()
    return git_blob == object_id


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def fetch_one(root: Path, relative: str, item: dict, *, remaining: int) -> dict:
    size = item.get("size")
    if not isinstance(size, int) or size < 0 or size > MAX_SINGLE_BYTES or size > remaining:
        raise ValueError(f"file exceeds declared byte limit: {relative} ({size})")
    target = safe_path(root, relative)
    if target.exists():
        if target.stat().st_size == size and verify_digest(target, item):
            return {"path": relative, "size_bytes": size, "sha256": sha256_file(target)}
        raise ValueError(f"existing file does not match pinned metadata: {target}")
    if shutil.disk_usage(root).free - size < MIN_FREE_BYTES:
        raise ValueError(f"less than {MIN_FREE_BYTES} bytes would remain free")
    target.parent.mkdir(parents=True, exist_ok=True)
    fd, part_name = tempfile.mkstemp(prefix=target.name + ".", suffix=".part", dir=target.parent)
    part = Path(part_name)
    digest = hashlib.sha256()
    received = 0
    started = time.monotonic()
    request = urllib.request.Request(BASE + relative, headers={"User-Agent": "crispmesh-bounded-subset/1"})
    try:
        with os.fdopen(fd, "wb") as output, urllib.request.urlopen(request, timeout=45) as response:
            length = response.headers.get("Content-Length")
            if length is not None and int(length) != size:
                raise ValueError(f"Content-Length mismatch for {relative}: {length} != {size}")
            while block := response.read(min(1024 * 1024, size - received + 1)):
                if time.monotonic() - started > MAX_FILE_SECONDS:
                    raise TimeoutError(f"download time limit exceeded: {relative}")
                received += len(block)
                if received > size or received > remaining:
                    raise ValueError(f"download exceeded byte limit: {relative}")
                output.write(block)
                digest.update(block)
        if received != size:
            raise ValueError(f"short download: {relative}: {received} != {size}")
        if not verify_digest(part, item):
            raise ValueError(f"source hash mismatch: {relative}")
        os.replace(part, target)
    finally:
        if part.exists():
            part.unlink()
    return {"path": relative, "size_bytes": size, "sha256": digest.hexdigest()}


def parse_cameras(path: Path) -> dict[int, dict]:
    cameras = {}
    for line in path.read_text().splitlines():
        if not line or line.startswith("#"):
            continue
        fields = line.split()
        camera_id, model, width, height = int(fields[0]), fields[1], int(fields[2]), int(fields[3])
        if camera_id in cameras:
            raise ValueError(f"duplicate camera ID {camera_id}")
        if model != "PINHOLE" or (width, height) != (5464, 3640):
            raise ValueError(f"unsupported camera {camera_id}: {model} {width}x{height}")
        params = [float(value) for value in fields[4:]]
        if len(params) != 4 or not all(math.isfinite(value) for value in params) or min(params[:2]) <= 0:
            raise ValueError(f"invalid intrinsics for camera {camera_id}")
        cameras[camera_id] = {"model": model, "width": width, "height": height,
                              "params": params}
    return cameras


def parse_images(path: Path) -> list[dict]:
    lines = [line for line in path.read_text().splitlines() if not line.startswith("#")]
    if len(lines) % 2:
        raise ValueError("COLMAP images.txt lacks an observation line")
    result = []
    seen_ids = set()
    seen_names = set()
    for index in range(0, len(lines), 2):
        fields = lines[index].split()
        if not fields:
            raise ValueError("blank COLMAP image record")
        if lines[index + 1].strip():
            raise ValueError("unexpected 2D observations; review selection method")
        if len(fields) != 10:
            raise ValueError("invalid COLMAP image record")
        name = fields[9]
        if PurePosixPath(name).name != name:
            raise ValueError(f"unsafe image name: {name}")
        image_id = int(fields[0])
        if image_id in seen_ids or name in seen_names:
            raise ValueError(f"duplicate image ID or name: {image_id} {name}")
        seen_ids.add(image_id)
        seen_names.add(name)
        qvec = [float(value) for value in fields[1:5]]
        tvec = [float(value) for value in fields[5:8]]
        if (not all(math.isfinite(value) for value in qvec + tvec) or
                abs(math.sqrt(sum(value*value for value in qvec)) - 1) > 1e-3):
            raise ValueError(f"invalid pose for image {image_id}")
        result.append({"id": image_id, "name": name,
                       "qvec": qvec,
                       "tvec": tvec,
                       "camera_id": int(fields[8])})
    return result


def geometry(image: dict) -> tuple[tuple[float, ...], tuple[float, ...]]:
    w, x, y, z = image["qvec"]
    tx, ty, tz = image["tvec"]
    # R maps world to camera; camera center and optical axis use R transpose.
    r00, r01, r02 = 1 - 2*(y*y+z*z), 2*(x*y-z*w), 2*(x*z+y*w)
    r10, r11, r12 = 2*(x*y+z*w), 1 - 2*(x*x+z*z), 2*(y*z-x*w)
    r20, r21, r22 = 2*(x*z-y*w), 2*(y*z+x*w), 1 - 2*(x*x+y*y)
    center = (-(r00*tx+r10*ty+r20*tz), -(r01*tx+r11*ty+r21*tz), -(r02*tx+r12*ty+r22*tz))
    direction = (r20, r21, r22)
    return center, direction


def select_images(images: list[dict], files: dict[str, dict], count: int = 10) -> list[dict]:
    eligible = [image for image in images if image["name"].startswith("The_Tree-")
                and image["name"] not in EXCLUDED and f'images/{image["name"]}' in files]
    if len(eligible) < count:
        raise ValueError("insufficient eligible cameras")
    poses = {image["id"]: geometry(image) for image in eligible}
    def affinity(a: dict, b: dict) -> float:
        ca, da = poses[a["id"]]
        cb, db = poses[b["id"]]
        distance = math.dist(ca, cb)
        facing = max(-1.0, min(1.0, sum(u*v for u, v in zip(da, db))))
        # Nearby camera centers and similarly oriented optical axes are a
        # useful prior for overlap, but are not measured image covisibility.
        # 2.0 is a dimensionless scale in the source reconstruction coordinates.
        return math.exp(-distance / 2.0) * max(0.0, facing) ** 4
    # Choose a well-connected seed, then a compact group around it.
    seeds = sorted(eligible, key=lambda image: (-sum(sorted(
        (affinity(image, other) for other in eligible if other is not image), reverse=True)[:count-1]), image["id"]))
    for seed in seeds:
        neighbors = sorted((image for image in eligible if image is not seed),
                           key=lambda image: (-affinity(seed, image), image["id"]))[:count-1]
        selected = [seed, *neighbors]
        if sum(files[f'images/{image["name"]}']["size"] for image in selected) <= MAX_PHOTO_BYTES:
            return selected
    raise ValueError("no pose-neighbor subset fits photo byte limit")


def run(root: Path, count: int) -> dict:
    if root.is_symlink():
        raise ValueError("output root may not be a symlink")
    root.mkdir(parents=True, exist_ok=True)
    files = api_files()
    base_names = ["LICENSE", "README.md", "colmap/cameras.txt", "colmap/images.txt", "manifest/images.csv"]
    if any(name not in files for name in base_names):
        raise ValueError("pinned revision is missing source metadata")
    # Reserve from the declared metadata before any file transfer.
    if sum(files[name]["size"] for name in base_names) > MAX_BATCH_BYTES:
        raise ValueError("source metadata exceeds batch byte limit")
    downloaded = []
    remaining = MAX_BATCH_BYTES
    for name in base_names:
        info = fetch_one(root, name, files[name], remaining=remaining)
        downloaded.append(info)
        remaining -= info["size_bytes"]
    cameras = parse_cameras(root / "colmap/cameras.txt")
    images = parse_images(root / "colmap/images.txt")
    selected = select_images(images, files, count=count)
    photo_bytes = sum(files[f'images/{image["name"]}']["size"] for image in selected)
    if photo_bytes > MAX_PHOTO_BYTES or photo_bytes > remaining:
        raise ValueError("selected photos exceed declared byte limit")
    records = []
    for image in selected:
        if image["camera_id"] not in cameras:
            raise ValueError(f'missing camera ID {image["camera_id"]}')
        name = f'images/{image["name"]}'
        info = fetch_one(root, name, files[name], remaining=remaining)
        remaining -= info["size_bytes"]
        records.append({**image, **info, "camera": cameras[image["camera_id"]]})
    manifest = {
        "schema_version": 1,
        "source": {"dataset": DATASET, "revision": REVISION, "license": "CC BY 4.0",
                   "credit": "Single Tree Photogrammetry Dataset — Matthew Guertin, 2020",
                   "url": f"https://huggingface.co/datasets/{DATASET}"},
        "colmap": {"cameras": "colmap/cameras.txt", "images": "colmap/images.txt", "points3D": None,
                   "tracks_available": False},
        "selection": {"method": "camera-center distance and optical-axis similarity heuristic",
                      "measured_covisibility": False, "excluded_suspect_names": sorted(EXCLUDED),
                      "count": len(records)},
        "source_files": downloaded,
        "selected_images": records,
        "total_source_bytes": sum(item["size_bytes"] for item in downloaded + records),
    }
    manifest_path = safe_path(root, "manifest.json")
    manifest_text = json.dumps(manifest, indent=2) + "\n"
    if manifest_path.exists():
        if manifest_path.read_text() != manifest_text:
            raise ValueError("existing manifest differs from pinned acquisition; refusing overwrite")
    else:
        with manifest_path.open("x") as output:
            output.write(manifest_text)
    return manifest


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=DEFAULT_ROOT)
    parser.add_argument("--count", type=int, default=10)
    args = parser.parse_args()
    if not 8 <= args.count <= 12:
        parser.error("--count must be 8–12")
    result = run(args.root, args.count)
    print(json.dumps({"manifest": str(args.root / "manifest.json"),
                      "photos": len(result["selected_images"]),
                      "total_source_bytes": result["total_source_bytes"]}, indent=2))
