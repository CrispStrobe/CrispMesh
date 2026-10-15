"""Read-only ID-bound diagnostic of the sealed GLOBAL mustard pose export.

Only the receipt and -V -I -E JSON are read. Explicit view id_pose links,
intrinsics and rotations use the reviewed OpenMVG v2.1 cereal parser.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from scripts.classical_backend import openmvg_mustard_json_pose_validator as prior
from scripts.classical_backend import openmvg_sceaux_postcheck as shared


OUTPUT = Path("/Volumes/backups/code/crispmesh-data/openmvg-mustard-photo-global-pose-export-001")
RECEIPT_SHA256 = "ac20487ca1932e7e42046c43b5fbae768383051f53a61729f4bb5d49dac7bebd"
JSON_SHA256 = "6382b68e07815d730f5b428e23004bac99a210fced8b5c9a5706e8503a2da709"
MODEL_SHA256 = "eb5d96ce4379ee32bdef3f946f833fd2c7a85b0a5c818faa4ce799766f74fc00"
SOURCE_RECEIPT_SHA256 = "612429ab5448fdf8bd71bcf6c03e6bafdedf2d38196dce8f51f27b277e8e19dc"
SFM_LOG_SHA256 = "5bed68eb709c8e534262b0f0a5f0689512bcf288db18eb15c8ee4d2a629b1f3d"
SOURCE_MODEL = OUTPUT.parent / "openmvg-mustard-photo-global-sfm-001/sparse/sfm_data.bin"
CONVERTER = OUTPUT.parent / "openmvg-v21-ligt-off-oracle-001/build/Darwin-arm64-Release/openMVG_main_ConvertSfM_DataFormat"
REPORT_COUNTS = {"views": 48, "poses": 46, "intrinsics": 1, "tracks": 69, "residuals": 1375}


def diagnose(output: Path = OUTPUT, *, expected_root: Path = OUTPUT,
             receipt_pin: str = RECEIPT_SHA256, json_pin: str = JSON_SHA256) -> dict:
    if output.resolve() != expected_root.resolve() or output.is_symlink() or not output.is_dir():
        raise ValueError("GLOBAL diagnostic accepts only the sealed export root")
    receipt_path = output / "receipt.json"
    json_path = output / "sfm_camera_poses.json"
    if (receipt_path.is_symlink() or not receipt_path.is_file() or
            receipt_path.stat().st_size > prior.MAX_JSON_BYTES or
            json_path.is_symlink() or not json_path.is_file() or
            not (0 < json_path.stat().st_size <= prior.MAX_JSON_BYTES)):
        raise ValueError("missing, linked, or oversized GLOBAL receipt/JSON")
    receipt_sha, json_sha = shared.sha256(receipt_path), shared.sha256(json_path)
    if receipt_sha != receipt_pin or json_sha != json_pin:
        raise ValueError("GLOBAL receipt or JSON differs from reviewed SHA-256")
    receipt = prior.load_json(receipt_path)
    model = receipt.get("global_model", {})
    exported = receipt.get("exported", {})
    expected_command = [str(CONVERTER), "-i", str(SOURCE_MODEL), "-o", str(json_path),
                        "-V", "-I", "-E"]
    if (receipt.get("schema") != "openmvg_mustard_global_pose_export_v1" or
            receipt.get("status") != "exported_global_model_diagnostic_only" or
            receipt.get("output") != str(output) or receipt.get("returncode") != 0 or
            receipt.get("json_sha256") != json_sha or
            receipt.get("json_bytes") != json_path.stat().st_size or
            receipt.get("command") != expected_command or
            receipt.get("converter", {}).get("binary_sha256") != prior.CONVERTER_BINARY_SHA256 or
            model.get("model_sha256") != MODEL_SHA256 or
            model.get("receipt_sha256") != SOURCE_RECEIPT_SHA256 or
            model.get("sfm_log_sha256") != SFM_LOG_SHA256 or
            model.get("source_receipt_sha256") != prior.SOURCE_RECEIPT_SHA256 or
            model.get("staged_match_count") != 105 or
            model.get("status") != "failed_registration_or_sparse_gate" or
            model.get("sfm_report") != REPORT_COUNTS or
            model.get("names") != list(prior.TRAIN_NAMES) or
            (exported.get("views"), exported.get("poses"), exported.get("intrinsics"),
             exported.get("structure"), exported.get("control_points")) != (48, 46, 1, 0, 0)):
        raise ValueError("GLOBAL receipt does not bind the sealed model/export")
    payload = prior.load_json(json_path)
    result = prior.parse_export(payload, prior.TRAIN_NAMES)
    if (exported.get("missing_names") != result["missing_pose_names"] or
            exported.get("posed_names") != sorted(set(prior.TRAIN_NAMES) - set(result["missing_pose_names"]))):
        raise ValueError("GLOBAL receipt named-pose inventory differs from JSON")
    if shared.sha256(receipt_path) != receipt_sha or shared.sha256(json_path) != json_sha:
        raise ValueError("GLOBAL receipt/JSON changed during read-only diagnostic")
    return {"schema": "openmvg_mustard_global_pose_diagnostic_v1",
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
