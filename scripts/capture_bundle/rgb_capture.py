#!/usr/bin/env python3
"""Hash-bound manifest for original JPEG RGB captures without camera assumptions."""

import argparse
import hashlib
import json
import math
import re
import shutil
from pathlib import Path, PurePosixPath

from PIL import Image


SCHEMA = "crispmesh_rgb_capture_bundle_v1"
MOTIONS = {"stationary_object_moving_camera", "moving_object", "unknown"}
POSE_SPACES = {"arkit_world_camera_to_world", "opencv_world_to_camera"}
MAX_IMAGES = 1000
MAX_IMAGE_BYTES = 100_000_000
MAX_PIXELS = 50_000_000
MAX_MANIFEST_BYTES = 2_000_000
MIN_FREE_BYTES = 10 * 1024 ** 3
SHA = re.compile(r"[0-9a-f]{64}\Z")


def sha256(path):
    hasher = hashlib.sha256()
    with Path(path).open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def safe_photo(root, name):
    if (not isinstance(name, str) or not name or len(name) > 240 or
            any(ord(char) < 32 for char in name) or ":" in name or "\\" in name or
            name != name.strip()):
        raise ValueError("invalid photo-relative path")
    pure = PurePosixPath(name)
    if (pure.is_absolute() or any(part in ("", ".", "..") for part in name.split("/")) or
            pure.suffix.lower() not in (".jpg", ".jpeg")):
        raise ValueError("photo must be a relative original JPEG path")
    root = Path(root)
    if root.is_symlink() or not root.is_dir():
        raise ValueError("photo root must be an existing real directory")
    current = root
    for part in pure.parts:
        current /= part
        if current.is_symlink():
            raise ValueError("photo path contains a symlink")
    root = root.resolve(strict=True)
    target = current.resolve(strict=True)
    if not target.is_relative_to(root) or not target.is_file():
        raise ValueError("photo path escapes root or is not a regular file")
    return target


def photo_record(root, name):
    path = safe_photo(root, name)
    size = path.stat().st_size
    if not 0 < size <= MAX_IMAGE_BYTES:
        raise ValueError("JPEG file size out of bounds")
    early_sha = sha256(path)
    with Image.open(path) as image:
        if image.format != "JPEG" or image.mode not in ("RGB", "L"):
            raise ValueError("JPEG RGB/grayscale decode required; HEIC is not supported")
        width, height = image.size
        if min(width, height) < 32 or width * height > MAX_PIXELS:
            raise ValueError("JPEG pixel dimensions out of bounds")
        orientation = image.getexif().get(274)
        if orientation is not None and (not isinstance(orientation, int) or isinstance(orientation, bool) or
                                        not 1 <= orientation <= 8):
            raise ValueError("invalid EXIF orientation")
        image.load()  # Decode but never transpose, normalize, save or copy the original.
    if path.stat().st_size != size or sha256(path) != early_sha:
        raise ValueError("photo changed during decode")
    return {"path": name, "bytes": size, "sha256": early_sha,
            "format": "JPEG", "stored_width": width, "stored_height": height,
            "exif_orientation": orientation}


def finite_number(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)


def validate_intrinsics(value, images):
    if value is None:
        return
    required = {"coordinate_space", "source", "width", "height", "fx", "fy", "cx", "cy",
                "distortion_model", "distortion"}
    if not isinstance(value, dict) or set(value) != required:
        raise ValueError("intrinsics fields must be explicit and complete")
    if value["coordinate_space"] != "stored_jpeg_pixels" or not isinstance(value["source"], str) or not value["source"].strip():
        raise ValueError("intrinsics must state stored-JPEG pixel coordinates and source")
    width, height = value["width"], value["height"]
    if (not isinstance(width, int) or isinstance(width, bool) or not isinstance(height, int) or
            isinstance(height, bool) or any((row["stored_width"], row["stored_height"]) != (width, height)
                                                for row in images)):
        raise ValueError("shared intrinsics must match every stored JPEG dimension")
    values = [value[key] for key in ("fx", "fy", "cx", "cy")]
    if (not all(finite_number(item) for item in values) or values[0] <= 0 or values[1] <= 0 or
            not 0 <= values[2] <= width or not 0 <= values[3] <= height):
        raise ValueError("invalid finite focal/principal-point values")
    distortion = value["distortion"]
    model = value["distortion_model"]
    if (not isinstance(model, str) or not isinstance(distortion, list) or
            model == "none" and distortion != [] or
            model == "opencv-radtan" and len(distortion) not in (4, 5, 8) or
            model not in ("none", "opencv-radtan") or not all(finite_number(item) for item in distortion)):
        raise ValueError("unsupported or nonfinite distortion")


def validate_matrix(matrix):
    if (not isinstance(matrix, list) or len(matrix) != 4 or
            any(not isinstance(row, list) or len(row) != 4 for row in matrix) or
            not all(finite_number(value) for row in matrix for value in row)):
        raise ValueError("pose must be finite 4x4 matrix")
    if any(abs(matrix[3][index] - expected) > 1e-8 for index, expected in enumerate((0, 0, 0, 1))):
        raise ValueError("pose homogeneous row invalid")
    rotation = [row[:3] for row in matrix[:3]]
    for i in range(3):
        for j in range(3):
            dot = sum(rotation[i][k] * rotation[j][k] for k in range(3))
            if abs(dot - (1 if i == j else 0)) > 1e-3:
                raise ValueError("pose rotation is not orthonormal")
    determinant = (rotation[0][0] * (rotation[1][1] * rotation[2][2] - rotation[1][2] * rotation[2][1]) -
                   rotation[0][1] * (rotation[1][0] * rotation[2][2] - rotation[1][2] * rotation[2][0]) +
                   rotation[0][2] * (rotation[1][0] * rotation[2][1] - rotation[1][1] * rotation[2][0]))
    if abs(determinant - 1) > 1e-3:
        raise ValueError("pose rotation must be proper")


def validate_poses(value, images):
    if value is None:
        return
    if (not isinstance(value, dict) or set(value) != {"coordinate_space", "translation_units", "source", "frames"} or
            not isinstance(value["coordinate_space"], str) or value["coordinate_space"] not in POSE_SPACES or
            value["translation_units"] not in ("meters", "unknown") or
            not isinstance(value["source"], str) or not value["source"].strip() or
            not isinstance(value["frames"], list) or len(value["frames"]) != len(images)):
        raise ValueError("pose metadata requires explicit space, units, source and all frames")
    names = [frame.get("path") for frame in value["frames"] if isinstance(frame, dict)]
    if (len(names) != len(images) or any(not isinstance(name, str) for name in names) or
            len(names) != len(set(names)) or
            set(names) != {image["path"] for image in images}):
        raise ValueError("pose frame names must exactly match JPEGs")
    for frame in value["frames"]:
        if set(frame) != {"path", "matrix4x4"}:
            raise ValueError("pose frame fields invalid")
        validate_matrix(frame["matrix4x4"])


def validate_structure(bundle):
    if (not isinstance(bundle, dict) or set(bundle) != {"schema", "capture_motion", "metric_scale_asserted",
                                                  "pixel_orientation_policy", "images", "intrinsics", "poses",
                                                  "pose_use_policy", "projection_compatibility",
                                                  "capture_motion_verified", "producer_sha256"} or
            bundle["schema"] != SCHEMA or not isinstance(bundle["capture_motion"], str) or
            bundle["capture_motion"] not in MOTIONS or
            bundle["metric_scale_asserted"] is not False or
            bundle["capture_motion_verified"] is not False or
            bundle["projection_compatibility"] != "unverified; EXIF orientation and camera axes not converted" or
            bundle["pixel_orientation_policy"] != "stored_jpeg_pixels; EXIF preserved; no transpose" or
            not isinstance(bundle["images"], list) or not 1 <= len(bundle["images"]) <= MAX_IMAGES or
            not isinstance(bundle["producer_sha256"], str) or not SHA.fullmatch(bundle["producer_sha256"])):
        raise ValueError("invalid RGB capture bundle structure or scale assertion")
    names, digests = [], []
    for row in bundle["images"]:
        if (not isinstance(row, dict) or set(row) != {"path", "bytes", "sha256", "format",
                                                        "stored_width", "stored_height", "exif_orientation"} or
                not isinstance(row["path"], str) or not row["path"] or len(row["path"]) > 240 or
                row["format"] != "JPEG" or not isinstance(row["sha256"], str) or not SHA.fullmatch(row["sha256"]) or
                not isinstance(row["bytes"], int) or isinstance(row["bytes"], bool) or
                not 0 < row["bytes"] <= MAX_IMAGE_BYTES or
                not isinstance(row["stored_width"], int) or isinstance(row["stored_width"], bool) or
                not isinstance(row["stored_height"], int) or isinstance(row["stored_height"], bool) or
                min(row["stored_width"], row["stored_height"]) < 32 or
                row["stored_width"] * row["stored_height"] > MAX_PIXELS or
                row["exif_orientation"] is not None and
                (not isinstance(row["exif_orientation"], int) or isinstance(row["exif_orientation"], bool) or
                 not 1 <= row["exif_orientation"] <= 8)):
            raise ValueError("invalid JPEG record")
        names.append(row["path"])
        digests.append(row["sha256"])
    if (len(names) != len(set(names)) or len(names) != len({name.casefold() for name in names}) or
            len(digests) != len(set(digests))):
        raise ValueError("duplicate JPEG name, case-folded name or content")
    validate_intrinsics(bundle["intrinsics"], bundle["images"])
    validate_poses(bundle["poses"], bundle["images"])
    expected_policy = ("ARKit/device camera poses are metadata only, never moving-object poses"
                       if bundle["capture_motion"] != "stationary_object_moving_camera"
                       else "camera poses require explicit coordinate conversion; no direct backend import")
    if bundle["pose_use_policy"] != expected_policy:
        raise ValueError("camera pose policy differs from declared capture motion")


def create(root, names, capture_motion, output, *, intrinsics=None, poses=None):
    if (not isinstance(capture_motion, str) or capture_motion not in MOTIONS or
            not isinstance(names, list) or not 1 <= len(names) <= MAX_IMAGES or
            any(not isinstance(name, str) for name in names)):
        raise ValueError("capture motion or image count invalid")
    if len(names) != len(set(names)) or len(names) != len({name.casefold() for name in names}):
        raise ValueError("duplicate image names")
    output = Path(output)
    if output.exists() or output.is_symlink():
        raise FileExistsError(output)
    output.parent.mkdir(parents=True, exist_ok=True)
    if shutil.disk_usage(output.parent).free < MIN_FREE_BYTES:
        raise RuntimeError("less than 10 GiB free")
    records = [photo_record(root, name) for name in names]
    policy = ("camera poses require explicit coordinate conversion; no direct backend import"
              if capture_motion == "stationary_object_moving_camera" else
              "ARKit/device camera poses are metadata only, never moving-object poses")
    bundle = {"schema": SCHEMA, "capture_motion": capture_motion,
              "capture_motion_verified": False,
              "metric_scale_asserted": False,
              "pixel_orientation_policy": "stored_jpeg_pixels; EXIF preserved; no transpose",
              "projection_compatibility": "unverified; EXIF orientation and camera axes not converted",
              "images": records, "intrinsics": intrinsics, "poses": poses,
              "pose_use_policy": policy, "producer_sha256": sha256(__file__)}
    validate_structure(bundle)
    for record in records:
        if photo_record(root, record["path"]) != record:
            raise ValueError("original JPEG changed before manifest publication")
    if sha256(__file__) != bundle["producer_sha256"]:
        raise ValueError("capture bundle helper changed during publication")
    payload = (json.dumps(bundle, indent=2, allow_nan=False) + "\n").encode()
    if len(payload) > MAX_MANIFEST_BYTES:
        raise ValueError("capture manifest exceeds 2 MB bound")
    with output.open("xb") as destination:
        destination.write(payload)
    return bundle


def strict_json(raw):
    def no_duplicates(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("duplicate JSON key")
            result[key] = value
        return result
    return json.loads(raw, object_pairs_hook=no_duplicates,
                      parse_constant=lambda value: (_ for _ in ()).throw(ValueError("nonfinite JSON")))


def load_metadata(path):
    if path is None:
        return None
    path = Path(path)
    if path.is_symlink() or not path.is_file() or path.stat().st_size > 1_000_000:
        raise ValueError("camera metadata JSON missing, symlinked or oversized")
    return strict_json(path.read_bytes())


def validate(root, manifest):
    manifest = Path(manifest)
    if manifest.is_symlink() or not manifest.is_file() or manifest.stat().st_size > MAX_MANIFEST_BYTES:
        raise ValueError("capture manifest missing, symlinked or oversized")
    raw = manifest.read_bytes()
    bundle = strict_json(raw)
    validate_structure(bundle)
    for record in bundle["images"]:
        if photo_record(root, record["path"]) != record:
            raise ValueError(f"original JPEG differs from manifest: {record['path']}")
    if manifest.read_bytes() != raw:
        raise ValueError("capture manifest changed during validation")
    return {"schema": "crispmesh_rgb_capture_validation_v1", "status": "valid",
            "manifest_sha256": hashlib.sha256(raw).hexdigest(),
            "image_count": len(bundle["images"]), "capture_motion": bundle["capture_motion"],
            "capture_motion_verified": False, "metric_scale_asserted": False,
            "projection_compatibility": bundle["projection_compatibility"],
            "images_copied_or_modified": 0,
            "pose_use_policy": bundle["pose_use_policy"]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    actions = parser.add_subparsers(dest="action", required=True)
    create_parser = actions.add_parser("create")
    create_parser.add_argument("--images-root", type=Path, required=True)
    create_parser.add_argument("--image", action="append", required=True,
                               help="source-relative JPEG path; repeat for each original")
    create_parser.add_argument("--capture-motion", choices=sorted(MOTIONS), required=True)
    create_parser.add_argument("--output", type=Path, required=True)
    create_parser.add_argument("--intrinsics-json", type=Path)
    create_parser.add_argument("--poses-json", type=Path)
    validation = actions.add_parser("validate")
    validation.add_argument("--images-root", type=Path, required=True)
    validation.add_argument("--manifest", type=Path, required=True)
    args = parser.parse_args()
    if args.action == "create":
        result = create(args.images_root, args.image, args.capture_motion, args.output,
                        intrinsics=load_metadata(args.intrinsics_json),
                        poses=load_metadata(args.poses_json))
        print(json.dumps({"status": "created", "image_count": len(result["images"]),
                          "metric_scale_asserted": False}))
    else:
        print(json.dumps(validate(args.images_root, args.manifest)))


if __name__ == "__main__":
    main()
