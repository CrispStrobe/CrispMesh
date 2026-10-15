"""Read-only ID-bound diagnostic of the sealed fixed-intrinsic mustard export.

Uses only the -003 conversion receipt and JSON, then reuses the reviewed
OpenMVG v2.1 cereal view/pose parser. No native process or reference is run.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from scripts.classical_backend import openmvg_mustard_json_pose_validator as prior
from scripts.classical_backend import openmvg_sceaux_postcheck as shared


OUTPUT = Path("/Volumes/backups/code/crispmesh-data/openmvg-mustard-photo-sfm-fixed-pose-export-003")
RECEIPT_SHA256 = "ba5450e939b824f1c529b53e65538d4d38ab5bb3bd8bf7f073b38015e2b50088"
JSON_SHA256 = "a9f6b6aa9ce63e163a9f1a324768a2593fefcdf10e3f149dc61c4ccd5d6c1854"
MODEL_SHA256 = "4147cd6ea9e0848dd97210701535371c100425125dc83df07a67482ebd8eca55"
SOURCE_RECEIPT_SHA256 = "5445b512ac6d171e8097134d9c86d84e6428b1d958a3088593d6234e5bb1fa24"
SFM_LOG_SHA256 = "7eaab977dddc6f1dc748fffc1d1c0affebf150bd762f56e333fdf9308898513d"
SOURCE_MODEL = OUTPUT.parent / "openmvg-mustard-photo-sfm-fixed-002/sparse/sfm_data.bin"
CONVERTER = OUTPUT.parent / "openmvg-v21-ligt-off-oracle-001/build/Darwin-arm64-Release/openMVG_main_ConvertSfM_DataFormat"
REPORT_COUNTS = {"views": 48, "poses": 46, "intrinsics": 1, "tracks": 162, "residuals": 1292}


def diagnose(output: Path = OUTPUT, *, expected_root: Path = OUTPUT,
             receipt_pin: str = RECEIPT_SHA256, json_pin: str = JSON_SHA256) -> dict:
    if output.resolve() != expected_root.resolve() or output.is_symlink() or not output.is_dir():
        raise ValueError("fixed-intrinsic diagnostic accepts only the sealed -003 root")
    receipt_path = output / "receipt.json"
    json_path = output / "sfm_camera_poses.json"
    if (receipt_path.is_symlink() or not receipt_path.is_file() or
            receipt_path.stat().st_size > prior.MAX_JSON_BYTES or
            json_path.is_symlink() or not json_path.is_file() or
            not (0 < json_path.stat().st_size <= prior.MAX_JSON_BYTES)):
        raise ValueError("missing, linked, or oversized -003 receipt/JSON")
    receipt_sha = shared.sha256(receipt_path)
    json_sha = shared.sha256(json_path)
    if receipt_sha != receipt_pin or json_sha != json_pin:
        raise ValueError("-003 receipt or JSON differs from reviewed SHA-256")
    receipt = prior.load_json(receipt_path)
    fixed = receipt.get("fixed_model", {})
    exported = receipt.get("exported", {})
    expected_command = [str(CONVERTER), "-i", str(SOURCE_MODEL), "-o", str(json_path),
                        "-V", "-I", "-E"]
    if (receipt.get("schema") != "openmvg_mustard_fixed_pose_export_v1" or
            receipt.get("status") != "exported_fixed_model_diagnostic_only" or
            receipt.get("output") != str(output) or receipt.get("returncode") != 0 or
            receipt.get("json_sha256") != json_sha or
            receipt.get("json_bytes") != json_path.stat().st_size or
            receipt.get("command") != expected_command or
            receipt.get("converter", {}).get("binary_sha256") != prior.CONVERTER_BINARY_SHA256 or
            fixed.get("model_sha256") != MODEL_SHA256 or
            fixed.get("receipt_sha256") != SOURCE_RECEIPT_SHA256 or
            fixed.get("sfm_log_sha256") != SFM_LOG_SHA256 or
            fixed.get("status") != "failed_registration_or_sparse_gate" or
            fixed.get("sfm_report") != REPORT_COUNTS or
            fixed.get("names") != list(prior.TRAIN_NAMES) or
            (exported.get("views"), exported.get("poses"), exported.get("intrinsics"),
             exported.get("structure"), exported.get("control_points")) != (48, 46, 1, 0, 0)):
        raise ValueError("-003 receipt does not bind the sealed fixed model/export")
    payload = prior.load_json(json_path)
    result = prior.parse_export(payload, prior.TRAIN_NAMES)
    if (exported.get("missing_names") != result["missing_pose_names"] or
            exported.get("posed_names") != sorted(set(prior.TRAIN_NAMES) - set(result["missing_pose_names"]))):
        raise ValueError("-003 receipt named-pose inventory differs from JSON")
    intrinsic = next(iter(result["intrinsics"].values()))
    if (intrinsic["focal_length"] != 1536.0 or
            intrinsic["principal_point"] != (640.0, 512.0) or
            intrinsic["distortion"].get("disto_k1") != (0.0,)):
        raise ValueError("fixed-intrinsic JSON changed the frozen image-only heuristic")
    if shared.sha256(receipt_path) != receipt_sha or shared.sha256(json_path) != json_sha:
        raise ValueError("-003 receipt/JSON changed during read-only diagnostic")
    return {"schema": "openmvg_mustard_fixed_pose_diagnostic_v1",
            "status": result["status"], "view_count": result["view_count"],
            "pose_count": result["pose_count"], "missing_pose_names": result["missing_pose_names"],
            "intrinsics": result["intrinsics"], "orbit": result["orbit"],
            "source_model_sha256": MODEL_SHA256,
            "export_receipt_sha256": receipt_sha, "export_json_sha256": json_sha,
            "unverified": result["unverified"]}


def main() -> None:
    argparse.ArgumentParser(description=__doc__).parse_args()
    print(json.dumps(diagnose(), sort_keys=True))


if __name__ == "__main__":
    main()
