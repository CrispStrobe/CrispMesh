"""Validate a future sealed OpenMVG v2.1 -V -I -E mustard JSON export.

No live export is accepted until its receipt SHA-256 and schema are reviewed
and pinned below. Pure payload validation is available for synthetic tests.
"""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
import re

from scripts.classical_backend import openmvg_mustard_sparse_diagnostic as sparse


EXPORT_ROOT = Path("/Volumes/backups/code/crispmesh-data/openmvg-mustard-photo-pose-export-001")
EXPORT_JSON = EXPORT_ROOT / "sfm_camera_poses.json"
EXPORT_RECEIPT = EXPORT_ROOT / "receipt.json"
APPROVED_RECEIPT_SHA256 = "ab73c2b729adb8abd1f84d97f1908b26635cd8caa6d2d1756185d9a31a34dac3"
APPROVED_RECEIPT_SCHEMA = "openmvg_mustard_pose_export_v1"
APPROVED_JSON_SHA256 = "c1e0ce4431245ca9392dd82b64cc8007d69e0c41a2e87cf78cec10f33ecc1124"
TRAIN_NAMES_SHA256 = sparse.TRAIN_NAMES_SHA
TRAIN_NAMES = tuple(f"NP3_{angle:03d}.jpg" for angle in range(0, 360, 6)
                    if angle % 30 != 24)
SOURCE_MODEL_SHA256 = "dc11f9b3a74809ebc080260b360ff1dd0e6f9f9526ea151aac002ace75b2df8b"
SOURCE_RECEIPT_SHA256 = "e45c400dcf71a3ba86fa22f7af2138418d6dc511fddcd6d8470ef33599804744"
CONVERTER_BINARY_SHA256 = "95bebb65afd1374aadae21aecc5f4cf66f4257432477d54ceae22c4cc35636b0"
UNDEFINED_INDEX = (1 << 32) - 1
MAX_JSON_BYTES = 2 << 20


def uint32(value, label: str, *, allow_undefined: bool = False) -> int:
    if type(value) is not int or value < 0 or value > UNDEFINED_INDEX or (not allow_undefined and value == UNDEFINED_INDEX):
        raise ValueError(f"invalid {label}")
    return value


def finite(value, label: str) -> float:
    if type(value) not in (int, float) or not math.isfinite(value):
        raise ValueError(f"nonfinite or nonnumeric {label}")
    return float(value)


def vector(value, length: int, label: str) -> tuple[float, ...]:
    if not isinstance(value, list) or len(value) != length:
        raise ValueError(f"malformed {label}")
    return tuple(finite(item, label) for item in value)


def rotation(value) -> tuple[tuple[float, ...], ...]:
    if not isinstance(value, list) or len(value) != 3:
        raise ValueError("malformed pose rotation")
    rows = tuple(vector(row, 3, "pose rotation") for row in value)
    for i, row in enumerate(rows):
        norm = sum(x * x for x in row)
        if abs(norm - 1.0) > 1e-5:
            raise ValueError("pose rotation is not orthonormal")
        for other in rows[:i]:
            if abs(sum(a * b for a, b in zip(row, other))) > 1e-5:
                raise ValueError("pose rotation is not orthonormal")
    a, b, c = rows
    determinant = (a[0] * (b[1] * c[2] - b[2] * c[1])
                   - a[1] * (b[0] * c[2] - b[2] * c[0])
                   + a[2] * (b[0] * c[1] - b[1] * c[0]))
    if abs(determinant - 1.0) > 1e-5:
        raise ValueError("pose rotation determinant is not +1")
    return rows


def entries(value, label: str) -> list[dict]:
    if not isinstance(value, list) or any(not isinstance(item, dict) for item in value):
        raise ValueError(f"malformed {label} list")
    return value


def load_json(path: Path) -> dict:
    def unique_pairs(pairs):
        value = {}
        for key, item in pairs:
            if key in value:
                raise ValueError(f"duplicate JSON key: {key}")
            value[key] = item
        return value

    def reject_constant(value):
        raise ValueError(f"nonfinite JSON constant: {value}")

    return json.loads(path.read_text(), object_pairs_hook=unique_pairs,
                      parse_constant=reject_constant)


def ptr_data(value: dict, label: str) -> dict:
    holder = value.get("value")
    wrapper = holder.get("ptr_wrapper") if isinstance(holder, dict) else None
    data = wrapper.get("data") if isinstance(wrapper, dict) else None
    if not isinstance(data, dict):
        raise ValueError(f"malformed cereal {label} pointer wrapper")
    return data


def parse_export(data: dict, train_names: tuple[str, ...]) -> dict:
    """Return named trajectory diagnostics from the exact v2.1 cereal JSON shape."""
    if (not isinstance(data, dict) or data.get("sfm_data_version") != "0.3" or
            not isinstance(data.get("root_path"), str) or
            len(train_names) != 48 or len(set(train_names)) != 48 or
            set(train_names) != set(TRAIN_NAMES) or
            any(not re.fullmatch(r"NP3_\d{3}\.jpg", name) for name in train_names)):
        raise ValueError("invalid JSON version or sealed TRAIN name inventory")
    if data.get("structure") not in (None, []) or data.get("control_points") not in (None, []):
        raise ValueError("export must contain only -V -I -E data")
    views = entries(data.get("views"), "views")
    intrinsics = entries(data.get("intrinsics"), "intrinsics")
    extrinsics = entries(data.get("extrinsics"), "extrinsics")
    if len(views) != 48 or len(intrinsics) != 1 or len(extrinsics) != 46:
        raise ValueError("JSON view/intrinsic/pose counts differ from sealed sparse run")
    cameras = {}
    for item in intrinsics:
        key = uint32(item.get("key"), "intrinsic key")
        if key in cameras:
            raise ValueError("duplicate intrinsic key")
        value = item.get("value")
        if not isinstance(value, dict) or not isinstance(value.get("polymorphic_name"), str):
            raise ValueError("malformed intrinsic type")
        fields = ptr_data(item, "intrinsic")
        width, height = fields.get("width"), fields.get("height")
        if type(width) is not int or type(height) is not int or (width, height) != (1280, 1024):
            raise ValueError("intrinsic dimensions differ from TRAIN JPEGs")
        focal = finite(fields.get("focal_length"), "focal length")
        principal = vector(fields.get("principal_point"), 2, "principal point")
        if focal <= 0 or not (0 <= principal[0] <= width and 0 <= principal[1] <= height):
            raise ValueError("invalid focal length or principal point")
        distortion = {}
        for field in ("disto_k1", "disto_k3", "disto_t2", "disto_k4"):
            if field in fields:
                values = fields[field]
                if not isinstance(values, list):
                    values = [values]
                distortion[field] = tuple(finite(number, field) for number in values)
        max_distortion = max((abs(number) for values in distortion.values() for number in values),
                             default=0.0)
        principal_offset = (abs(principal[0] - width / 2) / width,
                            abs(principal[1] - height / 2) / height)
        cameras[key] = {"type": value["polymorphic_name"], "focal_length": focal,
                        "principal_point": principal, "distortion": distortion,
                        "generic_sanity": {
                            "focal_over_width": focal / width,
                            "principal_offset_from_center_frame_fraction": principal_offset,
                            "max_abs_distortion_coefficient": max_distortion,
                            "focal_at_least_half_frame_width": focal / width >= .5,
                            "principal_within_central_half_frame": max(principal_offset) <= .25,
                            "abs_distortion_coefficients_at_most_one": max_distortion <= 1.0}}
    poses = {}
    for item in extrinsics:
        key = uint32(item.get("key"), "pose key")
        if key in poses or not isinstance(item.get("value"), dict):
            raise ValueError("duplicate or malformed pose key")
        value = item["value"]
        poses[key] = {"rotation": rotation(value.get("rotation")),
                      "center": vector(value.get("center"), 3, "pose center")}
    named = {}
    view_ids = set()
    used_pose_ids = set()
    for item in views:
        key = uint32(item.get("key"), "view key")
        value = ptr_data(item, "view")
        view_id = uint32(value.get("id_view"), "view id")
        pose_id = uint32(value.get("id_pose"), "view pose id", allow_undefined=True)
        intrinsic_id = uint32(value.get("id_intrinsic"), "view intrinsic id")
        name = value.get("filename")
        if (key != view_id or view_id in view_ids or name not in train_names or name in named or
                value.get("local_path") not in ("", ".") or intrinsic_id not in cameras or
                (value.get("width"), value.get("height")) != (1280, 1024)):
            raise ValueError("malformed, duplicate, or unsealed view")
        view_ids.add(view_id)
        record = {"id_view": view_id, "id_pose": pose_id, "id_intrinsic": intrinsic_id}
        if pose_id in poses:
            if pose_id in used_pose_ids:
                raise ValueError("two TRAIN views share one recovered pose")
            used_pose_ids.add(pose_id)
            record["center"] = poses[pose_id]["center"]
        named[name] = record
    if set(named) != set(train_names) or used_pose_ids != set(poses):
        raise ValueError("view names or pose links are incomplete")
    registered = {int(name[4:7]): row["center"] for name, row in named.items() if "center" in row}
    if len(registered) != 46:
        raise ValueError("named recovered pose count is not 46")
    orbit = sparse.orbit_diagnostic(registered)
    orbit["mapping"] = "explicit cereal view id_pose -> extrinsic key; TRAIN filename supplies angle"
    return {"schema": "openmvg_mustard_json_pose_check_v1", "status": "failed_48_of_48_gate",
            "view_count": 48, "pose_count": 46,
            "missing_pose_names": sorted(name for name, row in named.items() if "center" not in row),
            "intrinsics": cameras,
            "orbit": {**orbit, "status": "mapped_by_cereal_view_pose_ids"},
            "unverified": ["reference camera agreement", "mesh or physical accuracy",
                           "track membership and independent reprojections"]}


def audit(export_json: Path = EXPORT_JSON, receipt_path: Path = EXPORT_RECEIPT,
          *, receipt_pin: str | None = APPROVED_RECEIPT_SHA256,
          receipt_schema: str | None = APPROVED_RECEIPT_SCHEMA,
          expected_json_hash: str = APPROVED_JSON_SHA256,
          expected_json: Path = EXPORT_JSON,
          expected_receipt: Path = EXPORT_RECEIPT) -> dict:
    """Fail closed until the exact future conversion receipt is approved."""
    if receipt_pin is None or receipt_schema is None:
        return {"schema": "openmvg_mustard_json_pose_check_v1", "status": "abstained_unpinned_export",
                "reason": "approved conversion receipt SHA-256 and schema are unset"}
    if (export_json.resolve() != expected_json.resolve() or
            receipt_path.resolve() != expected_receipt.resolve() or
            receipt_path.is_symlink() or not receipt_path.is_file() or
            receipt_path.stat().st_size > MAX_JSON_BYTES or
            not export_json.is_file() or export_json.is_symlink() or
            export_json.stat().st_size > MAX_JSON_BYTES):
        raise ValueError("export paths or JSON size differ from reviewed contract")
    receipt_hash = sparse.shared.sha256(receipt_path)
    if receipt_hash != receipt_pin:
        raise ValueError("conversion receipt SHA-256 differs from approved pin")
    receipt = load_json(receipt_path)
    source = receipt.get("failed_model", {})
    exported = receipt.get("exported", {})
    command = receipt.get("command")
    expected_command = [str(EXPORT_ROOT.parent / "openmvg-v21-ligt-off-oracle-001" /
                            "build/Darwin-arm64-Release/openMVG_main_ConvertSfM_DataFormat"),
                        "-i", str(EXPORT_ROOT.parent / "openmvg-mustard-photo-sfm-001/sparse/sfm_data.bin"),
                        "-o", str(expected_json), "-V", "-I", "-E"]
    json_hash = sparse.shared.sha256(export_json)
    if (receipt.get("schema") != receipt_schema or
            receipt.get("status") != "exported_failed_model_diagnostic_only" or
            receipt.get("output") != str(expected_json.parent) or
            receipt.get("returncode") != 0 or receipt.get("json_bytes") != export_json.stat().st_size or
            receipt.get("json_sha256") != json_hash or json_hash != expected_json_hash or
            source.get("sfm_data_bin_sha256") != SOURCE_MODEL_SHA256 or
            source.get("receipt_sha256") != SOURCE_RECEIPT_SHA256 or
            source.get("status") != "failed_registration_or_sparse_gate" or
            source.get("sfm_report") != {"views": 48, "poses": 46, "intrinsics": 1,
                                         "tracks": 222, "residuals": 1497} or
            receipt.get("converter", {}).get("binary_sha256") != CONVERTER_BINARY_SHA256 or
            command != expected_command or
            exported.get("views") != 48 or exported.get("poses") != 46 or
            exported.get("intrinsics") != 1 or exported.get("structure") != 0 or
            exported.get("control_points") != 0):
        raise ValueError("conversion receipt does not bind approved model, TRAIN names, and JSON")
    data = load_json(export_json)
    result = parse_export(data, TRAIN_NAMES)
    if (exported.get("missing_names") != result["missing_pose_names"] or
            exported.get("posed_names") != sorted(set(TRAIN_NAMES) - set(result["missing_pose_names"]))):
        raise ValueError("export receipt named-pose inventory differs from JSON")
    if (sparse.shared.sha256(receipt_path) != receipt_hash or
            sparse.shared.sha256(export_json) != receipt["json_sha256"]):
        raise ValueError("conversion receipt/JSON changed during validation")
    result["export_json_sha256"] = receipt["json_sha256"]
    result["export_receipt_sha256"] = receipt_hash
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.parse_args()
    print(json.dumps(audit(), sort_keys=True))


if __name__ == "__main__":
    main()
