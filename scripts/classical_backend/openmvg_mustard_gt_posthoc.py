"""Posthoc, evaluation-only Berkeley camera comparison for two sealed SfM arms.

Default invocation is read-only and never opens H5 datasets. --run scores only
after separate review; reference metadata cannot enter either reconstruction.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import importlib.metadata
import importlib.util
import json
from pathlib import Path
import shutil
import tarfile
import time

from scripts.classical_backend import openmvg_mustard_json_pose_validator as poses

BASE = Path("/Volumes/backups/code/crispmesh-data")
OUTPUT = BASE / "openmvg-mustard-gt-posthoc-001"
ADJUST = BASE / "openmvg-mustard-photo-pose-export-001"
FIXED = BASE / "openmvg-mustard-photo-sfm-fixed-pose-export-003"
FIXED_SOURCE = BASE / "openmvg-mustard-photo-sfm-fixed-002"
FIXED_EXPORT_RECEIPT_SHA = "ba5450e939b824f1c529b53e65538d4d38ab5bb3bd8bf7f073b38015e2b50088"
FIXED_EXPORT_JSON_SHA = "a9f6b6aa9ce63e163a9f1a324768a2593fefcdf10e3f149dc61c4ccd5d6c1854"
FIXED_SOURCE_RECEIPT_SHA = "5445b512ac6d171e8097134d9c86d84e6428b1d958a3088593d6234e5bb1fa24"
FIXED_MODEL_SHA = "4147cd6ea9e0848dd97210701535371c100425125dc83df07a67482ebd8eca55"
CONVERTER_SHA = "95bebb65afd1374aadae21aecc5f4cf66f4257432477d54ceae22c4cc35636b0"
EXPECTED_NAMES = tuple(name for name in poses.TRAIN_NAMES if name not in
                       ("NP3_042.jpg", "NP3_048.jpg"))
PREFIX = "006_mustard_bottle/"
METADATA_ROOT = BASE / "mustard-pose-metadata-001"
METADATA = METADATA_ROOT / "metadata"
METADATA_RECEIPT = METADATA_ROOT / "receipt.json"
METADATA_RECEIPT_SHA = "3a2b156289b7f7f11d20ef5026ffe9f19893333d155f0acb6e326f027ceb0747"
ARCHIVE = METADATA_ROOT / "006_mustard_bottle_berkeley_rgbd.tgz"
ARCHIVE_SHA = "5d9b1837eb58b0760463e99021a53fe6e82d5cd2457141945445ed6df06ff3f7"
ARCHIVE_BYTES = 657272400
OUTPUT_CAP = 2 << 20
STAGED_CAP = 64 << 10
FLOOR = 11 << 30
WALL_SECONDS = 300
COMPARE_SOURCE = Path(__file__).resolve().parents[1] / "object_motion/checkerboard_pose_reference_eval.py"
COMPARE_SOURCE_SHA = "f24442884ecb9e102ee5ad96fb4727b56dc30e94d0f92aa783c6350704878c7b"


def sha(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def named_cameras(data: dict) -> tuple[dict, dict]:
    """Use the independently validated cereal id_pose linkage, never array order."""
    checked = poses.parse_export(data, poses.TRAIN_NAMES)
    if checked["missing_pose_names"] != ["NP3_042.jpg", "NP3_048.jpg"]:
        raise ValueError("exported common 46-name pose set differs")
    by_id = {row["key"]: row["value"] for row in data["extrinsics"]}
    centers, rotations = {}, {}
    for row in data["views"]:
        view = row["value"]["ptr_wrapper"]["data"]
        pose = by_id.get(view["id_pose"])
        if pose is None:
            continue
        name = view["filename"]
        centers[name] = pose["center"]
        rotations[name] = pose["rotation"]
    if set(centers) != set(rotations) or set(centers) != set(EXPECTED_NAMES):
        raise ValueError("two-arm named pose linkage differs from frozen common 46")
    return centers, rotations


def audit_exports() -> tuple[dict, dict, dict]:
    # Existing ADJUST_ALL validator binds its receipt, model, converter and JSON.
    adjust_check = poses.audit()
    if (adjust_check.get("export_receipt_sha256") != poses.APPROVED_RECEIPT_SHA256 or
            adjust_check.get("export_json_sha256") != poses.APPROVED_JSON_SHA256):
        raise ValueError("ADJUST_ALL export seal differs")
    adjust_data = poses.load_json(ADJUST / "sfm_camera_poses.json")
    adjust_centers, adjust_rotations = named_cameras(adjust_data)
    receipt_path = FIXED / "receipt.json"
    json_path = FIXED / "sfm_camera_poses.json"
    source_receipt = FIXED_SOURCE / "receipt.json"
    source_model = FIXED_SOURCE / "sparse/sfm_data.bin"
    expected = ((receipt_path, FIXED_EXPORT_RECEIPT_SHA), (json_path, FIXED_EXPORT_JSON_SHA),
                (source_receipt, FIXED_SOURCE_RECEIPT_SHA), (source_model, FIXED_MODEL_SHA))
    for path, digest in expected:
        if path.is_symlink() or not path.is_file() or sha(path) != digest:
            raise ValueError(f"fixed-intrinsic model/export seal differs: {path.name}")
    if json_path.stat().st_size > poses.MAX_JSON_BYTES:
        raise ValueError("fixed-intrinsic export JSON exceeds bound")
    receipt = poses.load_json(receipt_path)
    fixed_data = poses.load_json(json_path)
    fixed_centers, fixed_rotations = named_cameras(fixed_data)
    expected_command = [str(BASE / "openmvg-v21-ligt-off-oracle-001" /
                            "build/Darwin-arm64-Release/openMVG_main_ConvertSfM_DataFormat"),
                        "-i", str(source_model), "-o", str(json_path), "-V", "-I", "-E"]
    exported = receipt.get("exported", {})
    source = receipt.get("fixed_model", {})
    if (receipt.get("schema") != "openmvg_mustard_fixed_pose_export_v1" or
            receipt.get("status") != "exported_fixed_model_diagnostic_only" or
            receipt.get("output") != str(FIXED) or receipt.get("returncode") != 0 or
            receipt.get("json_sha256") != FIXED_EXPORT_JSON_SHA or
            receipt.get("json_bytes") != json_path.stat().st_size or
            receipt.get("command") != expected_command or
            source.get("receipt_sha256") != FIXED_SOURCE_RECEIPT_SHA or
            source.get("model_sha256") != FIXED_MODEL_SHA or
            source.get("sfm_report") != {"views": 48, "poses": 46, "intrinsics": 1,
                                         "tracks": 162, "residuals": 1292} or
            receipt.get("converter", {}).get("binary_sha256") != CONVERTER_SHA or
            exported.get("views") != 48 or exported.get("poses") != 46 or
            exported.get("intrinsics") != 1 or exported.get("structure") != 0 or
            exported.get("control_points") != 0 or
            exported.get("missing_names") != ["NP3_042.jpg", "NP3_048.jpg"] or
            exported.get("posed_names") != sorted(EXPECTED_NAMES)):
        raise ValueError("fixed-intrinsic export receipt linkage differs")
    return ({"receipt_sha256": poses.APPROVED_RECEIPT_SHA256,
             "json_sha256": poses.APPROVED_JSON_SHA256,
             "model_sha256": poses.SOURCE_MODEL_SHA256,
             "centers": adjust_centers, "rotations": adjust_rotations},
            {"receipt_sha256": FIXED_EXPORT_RECEIPT_SHA,
             "json_sha256": FIXED_EXPORT_JSON_SHA,
             "model_sha256": FIXED_MODEL_SHA,
             "source_receipt_sha256": FIXED_SOURCE_RECEIPT_SHA,
             "centers": fixed_centers, "rotations": fixed_rotations},
            {"common_names": list(EXPECTED_NAMES), "missing_names": ["NP3_042.jpg", "NP3_048.jpg"]})


def needed_metadata_members(names: tuple[str, ...] = EXPECTED_NAMES) -> set[str]:
    return {PREFIX + "calibration.h5"} | {
        PREFIX + f"poses/NP5_{int(name[4:7])}_pose.h5" for name in names}


def archive_inventory(archive: Path, needed: set[str], *, seconds: int = WALL_SECONDS) -> dict:
    started = time.monotonic()
    found = {}
    with tarfile.open(archive, "r|gz") as stream:
        for member in stream:
            if time.monotonic() - started > seconds:
                raise TimeoutError("cached archive inventory time cap exceeded")
            if member.name not in needed:
                continue
            if member.name in found or not member.isfile() or member.size != 2296:
                raise ValueError("duplicate/invalid cached NP5 metadata member")
            found[member.name] = {"bytes": member.size}
    if set(found) != needed:
        raise ValueError("cached archive lacks common-46 NP5 metadata")
    return found


def audit_metadata() -> dict:
    receipt_path = METADATA_RECEIPT
    if (receipt_path.is_symlink() or not receipt_path.is_file() or
            sha(receipt_path) != METADATA_RECEIPT_SHA or ARCHIVE.is_symlink() or
            not ARCHIVE.is_file() or ARCHIVE.stat().st_size != ARCHIVE_BYTES or
            sha(ARCHIVE) != ARCHIVE_SHA):
        raise ValueError("sealed Berkeley metadata receipt/archive differs")
    receipt = json.loads(receipt_path.read_text())
    found = receipt.get("extraction", {}).get("members", {})
    if (receipt.get("schema") != "mustard_pose_metadata_acquisition_v1" or
            receipt.get("status") != "metadata_only_no_camera_evaluation" or
            receipt.get("source", {}).get("archive", {}).get("sha256") != ARCHIVE_SHA or
            len(found) != 40 or receipt["extraction"].get("total_bytes", OUTPUT_CAP + 1) > OUTPUT_CAP):
        raise ValueError("sealed Berkeley extraction contract differs")
    needed = needed_metadata_members()
    extracted = needed & set(found)
    missing = needed - extracted
    if len(extracted) != 38 or len(missing) != 9 or PREFIX + "calibration.h5" not in extracted:
        raise ValueError("common-46 extracted/archive partition differs")
    for member in found:
        record = found[member]
        path = METADATA / Path(member).relative_to(PREFIX)
        if (not member.startswith(PREFIX) or path.is_symlink() or not path.is_file() or
                path.stat().st_size != record.get("bytes") or sha(path) != record.get("sha256")):
            raise ValueError(f"sealed extracted metadata differs: {member}")
    return {"receipt_sha256": METADATA_RECEIPT_SHA,
            "archive_sha256": ARCHIVE_SHA,
            "extracted_members": {name: found[name] for name in sorted(extracted)},
            "archive_only_members": archive_inventory(ARCHIVE, missing)}


def capacity(output: Path, prospective: bool = False) -> dict:
    external = shutil.disk_usage(BASE).free
    internal = shutil.disk_usage(Path(__file__).resolve().parents[2]).free
    used = sum(path.stat().st_size for path in output.rglob("*") if path.is_file()) if output.exists() else 0
    required = FLOOR + (OUTPUT_CAP - used if prospective else 0)
    if used > OUTPUT_CAP or external < required or internal < FLOOR:
        raise RuntimeError("posthoc comparison disk/output cap breached")
    return {"output_bytes": used, "external_free": external,
            "external_required": required, "internal_free": internal}


def algorithm_runtime() -> dict:
    if (COMPARE_SOURCE.is_symlink() or not COMPARE_SOURCE.is_file() or
            sha(COMPARE_SOURCE) != COMPARE_SOURCE_SHA):
        raise ValueError("shared Sim3/LOO source seal differs")
    if importlib.util.find_spec("numpy") is None or shutil.which("h5dump") is None:
        raise RuntimeError("NumPy and h5dump are required for the reviewed scoring runtime")
    return {"compare_source_sha256": COMPARE_SOURCE_SHA,
            "numpy_version": importlib.metadata.version("numpy"),
            "h5dump_path": shutil.which("h5dump")}


def preflight(output: Path = OUTPUT) -> dict:
    if (output.exists() or output.is_symlink() or output.parent != BASE or
            output.name != OUTPUT.name or BASE.is_symlink() or not BASE.is_dir()):
        raise ValueError("posthoc output must be the fresh reviewed external root")
    adjust, fixed, names = audit_exports()
    metadata = audit_metadata()
    for arm in (adjust, fixed):
        arm.pop("centers")
        arm.pop("rotations")
    return {"schema": "openmvg_mustard_two_arm_gt_posthoc_v1",
            "status": "read_only_preflight_no_gt_scoring", "output": str(output),
            "adjust_all": adjust, "fixed_none": fixed, "names": names,
            "metadata": metadata, "algorithm_runtime": algorithm_runtime(),
            "capacity": capacity(output, prospective=True),
            "limits": {"output_bytes": OUTPUT_CAP, "staged_metadata_bytes": STAGED_CAP,
                       "disk_floor_bytes": FLOOR, "wall_seconds": WALL_SECONDS},
            "separation": "posthoc only: no reference input to either SfM reconstruction",
            "license_scope": "evaluation only; no shipping or GPL/AGPL clearance"}


def stage_missing(output: Path, expected: dict) -> dict:
    staged = {}
    total = 0
    started = time.monotonic()
    with tarfile.open(ARCHIVE, "r|gz") as stream:
        for member in stream:
            if time.monotonic() - started > WALL_SECONDS:
                raise TimeoutError("cached metadata staging time cap exceeded")
            if member.name not in expected:
                continue
            if member.name in staged or not member.isfile() or member.size != 2296:
                raise ValueError("duplicate/invalid selected NP5 member")
            total += member.size
            if total > STAGED_CAP:
                raise ValueError("staged NP5 metadata cap exceeded")
            target = output / "metadata" / Path(member.name).name
            source = stream.extractfile(member)
            if source is None:
                raise ValueError("selected NP5 member unreadable")
            content = source.read(member.size + 1)
            if len(content) != member.size:
                raise ValueError("selected NP5 member size differs")
            target.write_bytes(content)
            staged[member.name] = {"bytes": len(content),
                                   "sha256": hashlib.sha256(content).hexdigest()}
            capacity(output)
    if set(staged) != set(expected):
        raise ValueError("selected NP5 archive inventory incomplete")
    verify_staged(output, staged)
    return staged


def verify_staged(output: Path, staged: dict) -> None:
    folder = output / "metadata"
    if (folder.is_symlink() or not folder.is_dir() or
            {path.name for path in folder.iterdir()} !=
            {Path(member).name for member in staged}):
        raise ValueError("staged NP5 metadata inventory differs")
    for member, record in staged.items():
        path = folder / Path(member).name
        if (path.is_symlink() or not path.is_file() or
                path.stat().st_size != record["bytes"] or sha(path) != record["sha256"]):
            raise ValueError(f"staged NP5 metadata hash differs: {member}")


def reference_cameras(output: Path, metadata: dict, started: float | None = None) -> tuple[dict, dict, dict]:
    import numpy as np
    from scripts.object_motion import checkerboard_pose_reference_eval as shared
    from scripts.object_motion.ycb_camera_reference import dataset, rigid
    calib = METADATA / "calibration.h5"
    np3_from_np5 = rigid(dataset(calib, "/H_NP3_from_NP5", (4, 4)), "mustard NP3 calibration")
    k = dataset(calib, "/NP3_rgb_K", (3, 3))
    d = dataset(calib, "/NP3_rgb_d", (5,))
    if not np.allclose(k[2], [0, 0, 1], atol=1e-10) or min(k[0, 0], k[1, 1]) <= 0:
        raise ValueError("invalid supplied NP3 RGB calibration")
    centers, rotations = {}, {}
    for name in EXPECTED_NAMES:
        if started is not None and time.monotonic() - started > WALL_SECONDS:
            raise TimeoutError("posthoc metadata read wall-time cap exceeded")
        capacity(output)
        member = PREFIX + f"poses/NP5_{int(name[4:7])}_pose.h5"
        path = (METADATA / "poses" / Path(member).name if member in metadata["extracted_members"]
                else output / "metadata" / Path(member).name)
        table_from_ref = rigid(dataset(path, "/H_table_from_reference_camera", (4, 4)), name)
        np3_from_table = rigid(np3_from_np5 @ np.linalg.inv(table_from_ref), name + " NP3-from-table")
        rotations[name] = np3_from_table[:3, :3]
        centers[name] = shared.camera_center(np3_from_table[:3, :3], np3_from_table[:3, 3])
    return centers, rotations, {"K": k.tolist(), "distortion": d.tolist()}


def compare_arms(adjust: dict, fixed: dict, gt_centers: dict, gt_rotations: dict) -> dict:
    from scripts.object_motion import checkerboard_pose_reference_eval as shared
    arms = {}
    for label, arm in (("adjust_all", adjust), ("fixed_none", fixed)):
        arms[label] = shared.compare(arm["centers"], arm["rotations"], gt_centers, gt_rotations)
        if arms[label]["matched_names"] != sorted(EXPECTED_NAMES):
            raise ValueError("posthoc Sim3 did not match frozen common 46")
        # The shared checkerboard evaluator's historical "square" label does
        # not apply to OpenMVG's arbitrary reconstruction scale.
        fit = arms[label]["fit"]
        fit["scale_berkeley_pose_units_per_openmvg_unit"] = fit.pop("scale_supplied_units_per_square")
        fit["world_translation_berkeley_pose_units"] = fit.pop("world_translation_supplied_units")
        fit["berkeley_pose_median_center_radius"] = fit.pop("supplied_median_center_radius")
        arms[label]["center_residual_berkeley_pose_units"] = arms[label].pop("center_residual_supplied_units")
        for row in arms[label]["per_frame"].values():
            row["center_residual_berkeley_pose_units"] = row.pop("center_residual_supplied_units")
    a, b = arms["adjust_all"], arms["fixed_none"]
    fields = ("center_residual_over_radius", "leave_one_out_center_residual_over_radius",
              "orientation_residual_degrees")
    deltas = {name: {field + "_fixed_minus_adjust": b["per_frame"][name][field] -
                     a["per_frame"][name][field] for field in fields}
              for name in sorted(EXPECTED_NAMES)}
    return {"arms": arms, "paired_per_frame_deltas": deltas,
            "delta_sign": "fixed NONE minus ADJUST_ALL; each arm has its own proper positive-scale Sim3 fit"}


def run(output: Path = OUTPUT) -> dict:
    checked = preflight(output)
    output.mkdir()
    (output / "metadata").mkdir()
    receipt = {**checked, "status": "running", "started_utc": datetime.now(timezone.utc).isoformat()}
    (output / "receipt.json").write_text(json.dumps(receipt, sort_keys=True, indent=2) + "\n")
    started = time.monotonic()
    try:
        staged = stage_missing(output, checked["metadata"]["archive_only_members"])
        receipt["staged_metadata"] = staged
        adjust, fixed, _ = audit_exports()
        if audit_metadata() != checked["metadata"]:
            raise ValueError("sealed metadata changed during posthoc scoring")
        target_centers, target_rotations, calibration = reference_cameras(output, checked["metadata"], started)
        comparison = compare_arms(adjust, fixed, target_centers, target_rotations)
        post_adjust, post_fixed, post_names = audit_exports()
        if ({key: post_adjust[key] for key in checked["adjust_all"]} != checked["adjust_all"] or
                {key: post_fixed[key] for key in checked["fixed_none"]} != checked["fixed_none"] or
                post_names != checked["names"] or audit_metadata() != checked["metadata"] or
                algorithm_runtime() != checked["algorithm_runtime"]):
            raise ValueError("sealed exports or Berkeley metadata changed during scoring")
        verify_staged(output, staged)
        if time.monotonic() - started > WALL_SECONDS:
            raise TimeoutError("posthoc comparison wall-time cap exceeded")
        report = {"schema": "openmvg_mustard_two_arm_gt_posthoc_result_v1",
                  "status": "posthoc_46_view_diagnostic_only", "inputs": checked,
                  "fit_units": "source centers: arbitrary OpenMVG units; target centers: Berkeley pose-translation units (not independently calibrated as meters or board squares)",
                  "reference_calibration_diagnostic": calibration,
                  "transform_convention": "H_NP3_from_table = H_NP3_from_NP5 @ inverse(H_table_from_reference_camera); world-to-camera OpenCV axes",
                  "caveats": ["Both models remain 46/48 incomplete and may have wrong geometry.",
                              "One Sim3 per arm removes arbitrary scale/origin/orientation; it does not prove absolute frame or mesh accuracy.",
                              "Orientation comparison is conditional on NP3/NP5 matrix-name and axis conventions; board_frame_offset/depth were not independently validated.",
                              "A nearly planar orbit weakens third-axis alignment; inspect covariance singular values and LOO errors.",
                              "Berkeley K and poses were opened only after SfM/export sealing, never fed to reconstruction."],
                  "comparison": comparison}
        encoded = (json.dumps(report, sort_keys=True, indent=2) + "\n").encode()
        if len(encoded) > OUTPUT_CAP - STAGED_CAP:
            raise ValueError("posthoc report exceeds small output cap")
        with (output / "report.json").open("xb") as stream:
            stream.write(encoded)
        capacity(output)
        receipt["report_sha256"] = sha(output / "report.json")
        receipt["report_bytes"] = len(encoded)
        receipt["status"] = "posthoc_46_view_diagnostic_only"
    except BaseException as exc:
        receipt["status"] = "stopped"
        receipt["reason"] = str(exc)
        raise
    finally:
        receipt["finished_utc"] = datetime.now(timezone.utc).isoformat()
        receipt["wall_seconds"] = round(time.monotonic() - started, 3)
        receipt["artifact_inventory"] = {
            str(path.relative_to(output)): {"bytes": path.stat().st_size, "sha256": sha(path)}
            for path in output.rglob("*") if path.is_file() and path != output / "receipt.json"}
        receipt["output_bytes_before_final_receipt"] = sum(
            path.stat().st_size for path in output.rglob("*") if path.is_file())
        (output / "receipt.json").write_text(json.dumps(receipt, sort_keys=True, indent=2) + "\n")
        capacity(output)
    return receipt


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", action="store_true", help="posthoc GT score; requires separate review")
    args = parser.parse_args()
    print(json.dumps(run() if args.run else preflight(), sort_keys=True, indent=2))


if __name__ == "__main__":
    main()
