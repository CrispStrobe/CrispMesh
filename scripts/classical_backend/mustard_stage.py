"""Hash-bound 48-train mustard staging gate; no SfM is launched here."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import time

from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
MOUNT = Path("/Volumes/backups")
OUTPUT = MOUNT / "code/crispmesh-data/mustard-sfm-train-001"
PACKAGE = ROOT / "build-opencv/ycb-evaluation-vps-001/mustard-package.json"
PACKAGE_SHA256 = "45b6ed1430627a747eaeb8d4dff7d2c49502083cca0a7914e25aacd45b594425"
ACQUISITION_SHA256 = "c48998dffc7798e7d21f275383b1c67dc2e5e9313a8bfe699f72303f61583b52"
PROMPTS_SHA256 = "28440795d5f38590df8c5568969970d6623fbe22ea6b3e8781cba10608149d32"
TRAIN_NAMES = tuple(f"NP3_{angle:03d}.jpg" for angle in range(0, 360, 6) if angle % 30 != 24)
HELDOUT_NAMES = tuple(f"NP3_{angle:03d}.jpg" for angle in range(24, 360, 30))
MAX_BYTES = 150 * 1024**2
MIN_FREE = 10 * 1024**3
BUFFER = 256 * 1024**2
MAX_SECONDS = 240


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def sealed_json(path: Path, digest: str) -> dict:
    if (len(digest) != 64 or any(char not in "0123456789abcdef" for char in digest) or
            path.is_symlink() or not path.is_file() or path.stat().st_size > 1024**2 or
            sha256(path) != digest):
        raise ValueError(f"missing, oversized, or changed sealed JSON: {path}")
    return json.loads(path.read_text())


def exact_files(directory: Path, expected: set[str]) -> None:
    if directory.is_symlink() or not directory.is_dir():
        raise ValueError(f"missing/linked train-only directory: {directory}")
    items = list(directory.iterdir())
    if {item.name for item in items} != expected or any(item.is_symlink() or not item.is_file() for item in items):
        raise ValueError("train-only directory has missing, extra, held-out, or linked files")


def mask_pixels(mask: Path, photo: Path, expected_size: tuple[int, int]) -> dict:
    with Image.open(photo) as image, Image.open(mask) as alpha:
        image.load()
        alpha.load()
        if (image.format != "JPEG" or image.mode != "RGB" or image.size != expected_size or
                alpha.format != "PNG" or alpha.mode != "L" or alpha.size != expected_size):
            raise ValueError("mustard photo/mask format or dimensions differ")
        histogram = alpha.histogram()
        if sum(histogram[1:255]) or not histogram[0] or not histogram[255]:
            raise ValueError("cleaned mask must contain both binary 0 and 255 only")
        return {"kept_pixels": histogram[255], "ignored_pixels": histogram[0]}


def validate_composed_candidate(inventory: dict, rows: list[dict], qa: dict,
                                inventory_root: Path, inventory_sha256: str,
                                package_sha256: str) -> None:
    """Additional gate for the distinct 43-base/5-corrected candidate lane."""
    from scripts.classical_backend import mustard_candidate48 as candidate

    corrected = set(candidate.CORRECTED)
    if (inventory.get("status") != "generated_unreviewed" or
            inventory.get("package_sha256") != package_sha256 or
            inventory.get("base_inventory_sha256") != candidate.BASE_SHA or
            inventory.get("base_rejected_qa_sha256") != candidate.REJECTED_QA_SHA or
            inventory.get("correction_prompt_sha256") == PROMPTS_SHA256 or
            inventory.get("full_training_package_ready") is not False or
            [row.get("name") for row in rows] != list(TRAIN_NAMES)):
        raise ValueError("composed 48-view inventory lineage differs")
    for key in ("correction_manifest_sha256", "correction_prompt_sha256",
                "correction_qa_sha256", "correction_supervisor_sha256", "composition_report_sha256"):
        value = inventory.get(key)
        if not isinstance(value, str) or len(value) != 64 or set(value) - set("0123456789abcdef"):
            raise ValueError("composed inventory lacks sealed correction lineage")
    report = sealed_json(inventory_root / "composition-report.json", inventory["composition_report_sha256"])
    if (report.get("schema") != "sam21_mustard_m1_composition_report_v1" or
            report.get("status") != "complete_unreviewed" or
            any(report.get(key) != inventory.get(key) for key in
                ("base_inventory_sha256", "base_rejected_qa_sha256", "correction_manifest_sha256",
                 "correction_prompt_sha256", "correction_qa_sha256", "correction_supervisor_sha256", "package_sha256")) or
            len(report.get("images", [])) != 48):
        raise ValueError("composition report does not seal this inventory")
    if (report.get("output") != str(inventory_root) or
            report.get("runner_sha256") != sha256(Path(candidate.__file__))):
        raise ValueError("composition output or runner seal differs")
    for row, source in zip(rows, report["images"]):
        name = row["name"]
        expected_origin = "board5" if name in corrected else "base43"
        if (row.get("origin") != expected_origin or source.get("origin") != expected_origin or
                row.get("prompt_sha256") != (inventory["correction_prompt_sha256"] if name in corrected else PROMPTS_SHA256) or
                any(row.get(key) != source.get(key) for key in
                    ("name", "source_sha256", "cleaned_mask_sha256", "origin",
                     "source_frame_manifest_sha256", "prompt_sha256", "prompt_points_sha256"))):
            raise ValueError(f"composed per-image base/correction provenance differs: {name}")
        for key in ("source_frame_manifest_sha256", "prompt_points_sha256"):
            value = row.get(key)
            if not isinstance(value, str) or len(value) != 64 or set(value) - set("0123456789abcdef"):
                raise ValueError("composed row lacks exact prompt/frame hash")
    if (qa.get("schema") != "mustard_sam_composed_candidate_qa_v1" or
            qa.get("status") != "accepted" or
            qa.get("decision") != "accepted_for_coarse_pose_support" or
            qa.get("inventory_sha256") != inventory_sha256 or
            qa.get("reviewed_names") != list(TRAIN_NAMES) or
            not qa.get("reviewer") or not qa.get("reviewed_at")):
        raise ValueError("independent full-48 composed candidate QA acceptance missing")
    audit = report.get("source_audit")
    if (not isinstance(audit, list) or len(audit) != 161 or
            any(not isinstance(pair, list) or len(pair) != 2 or
                not isinstance(pair[0], str) or not isinstance(pair[1], str) or
                len(pair[1]) != 64 or set(pair[1]) - set("0123456789abcdef")
                for pair in audit)):
        raise ValueError("composition did not bind all metadata and base/correction masks")
    base_root = Path(audit[1][0]).parent
    correction_root = Path(audit[3][0]).parent
    if (Path(audit[1][0]).name != "complete_inventory.json" or
            Path(audit[3][0]).name != "manifest.json" or
            Path(audit[6][0]) != correction_root / "supervisor.json" or
            audit[1][1] != inventory["base_inventory_sha256"] or
            audit[2][1] != inventory["base_rejected_qa_sha256"] or
            audit[3][1] != inventory["correction_manifest_sha256"] or
            audit[4][1] != inventory["correction_prompt_sha256"] or
            audit[5][1] != inventory["correction_qa_sha256"] or
            audit[6][1] != inventory["correction_supervisor_sha256"] or
            base_root.is_symlink() or correction_root.is_symlink()):
        raise ValueError("composition metadata audit paths or hashes differ")
    original_inventory = sealed_json(Path(audit[1][0]), audit[1][1])
    original_rows = original_inventory.get("images", [])
    if [r.get("name") for r in original_rows] != list(TRAIN_NAMES):
        raise ValueError("composition base audit inventory order differs")
    cursor = 7
    for row, original in zip(rows, original_rows):
        name = row["name"]
        expected = [base_root / "frames" / name / "frame.json",
                    base_root / "frames" / name / "raw.png",
                    base_root / "frames" / name / "clean.png"]
        if name in corrected:
            expected.extend((correction_root / "raw_masks" / (name + ".png"),
                             correction_root / "cleaned_masks" / (name + ".png")))
        if [Path(pair[0]) for pair in audit[cursor:cursor + len(expected)]] != expected:
            raise ValueError("composition per-frame source audit path differs")
        if (audit[cursor][1] != original.get("frame_manifest_sha256") or
                audit[cursor + 2][1] != original.get("cleaned_mask_sha256")):
            raise ValueError("composition base frame audit hash differs")
        cursor += len(expected)
    if cursor != len(audit):
        raise ValueError("composition source audit has extra entries")
    for source, expected_hash in audit:
        path = Path(source)
        if path.is_symlink() or not path.is_file() or sha256(path) != expected_hash:
            raise ValueError("composition source or metadata changed after review")


def validate_composed_selection(inventory: dict, rows: list[dict], qa: dict,
                                inventory_root: Path, inventory_sha256: str,
                                package_sha256: str) -> None:
    """Distinct 43+3+2 lane: preserve failed parent status and all per-view seals."""
    from scripts.classical_backend import mustard_candidate48 as candidate

    if (inventory.get("status") != "generated_unreviewed" or
            inventory.get("package_sha256") != package_sha256 or
            inventory.get("base_inventory_sha256") != candidate.BASE_SHA or
            inventory.get("base_rejected_qa_sha256") != candidate.REJECTED_QA_SHA or
            inventory.get("full_training_package_ready") is not False or
            [r.get("name") for r in rows] != list(TRAIN_NAMES) or
            inventory.get("parent_status_disclosure") != {
                "board5_partial3_manifest_status": "partial_unusable",
                "board5_partial3_supervisor_status": "failed",
                "board5_partial3_supervisor_reason": "worker exit 1"}):
        raise ValueError("composed 43+3+2 inventory lineage/status differs")
    for key in ("selection_sha256", "composition_report_sha256"):
        value = inventory.get(key)
        if not isinstance(value, str) or len(value) != 64 or set(value) - set("0123456789abcdef"):
            raise ValueError("composed 43+3+2 inventory lacks sealed selection/report")
    report = sealed_json(inventory_root / "composition-report.json", inventory["composition_report_sha256"])
    if (report.get("schema") != "sam21_mustard_m1_composition_report_v2" or
            report.get("status") != "complete_unreviewed" or
            report.get("output") != str(inventory_root) or
            report.get("runner_sha256") != sha256(Path(candidate.__file__)) or
            report.get("selection_sha256") != inventory["selection_sha256"] or
            report.get("contact_sheet_sha256") != inventory.get("contact_sheet_sha256") or
            report.get("contact_sheet_path") != "full48-contact-sheet.png" or
            report.get("parent_status_disclosure") != inventory["parent_status_disclosure"] or
            len(report.get("images", [])) != 48):
        raise ValueError("composed 43+3+2 report does not seal this inventory")
    sheet = inventory_root / "full48-contact-sheet.png"
    if (sheet.is_symlink() or not sheet.is_file() or
            inventory.get("contact_sheet_path") != sheet.name or
            sha256(sheet) != inventory.get("contact_sheet_sha256")):
        raise ValueError("composed full-frame contact sheet differs")
    origins = {"base43": 0, "board5_partial3": 0, "roi_v2_2": 0}
    shared = ("name", "origin", "source_sha256", "cleaned_mask_sha256",
              "source_frame_manifest_sha256", "prompt_sha256", "prompt_row_sha256",
              "parent_supervisor_sha256", "parent_manifest_status", "parent_supervisor_status",
              "parent_supervisor_reason", "per_view_qa_sha256", "resource_audit_sha256")
    for row, source in zip(rows, report["images"]):
        origin = "base43" if row.get("name") not in candidate.CORRECTED else (
            "board5_partial3" if row["name"] in candidate.PARTIAL_NAMES else "roi_v2_2")
        if (row.get("origin") != origin or any(row.get(k) != source.get(k) for k in shared) or
                row.get("cleaned_mask_path") != f"frames/{row['name']}/clean.png"):
            raise ValueError("composed 43+3+2 per-view row/source differs")
        origins[origin] += 1
        for key in ("source_frame_manifest_sha256", "prompt_sha256", "prompt_row_sha256"):
            value = row.get(key)
            if not isinstance(value, str) or len(value) != 64 or set(value) - set("0123456789abcdef"):
                raise ValueError("composed 43+3+2 row has missing lineage hash")
        if origin == "base43":
            if (row["prompt_sha256"] != PROMPTS_SHA256 or row.get("per_view_qa_sha256") is not None or
                    row.get("parent_supervisor_sha256") is not None):
                raise ValueError("base43 was relabelled as corrected")
        else:
            for key in ("parent_supervisor_sha256", "per_view_qa_sha256"):
                value = row.get(key)
                if not isinstance(value, str) or len(value) != 64 or set(value) - set("0123456789abcdef"):
                    raise ValueError("corrected row lacks parent/per-view QA seal")
            if origin == "board5_partial3" and (row.get("parent_manifest_status") != "partial_unusable" or
                    row.get("parent_supervisor_status") != "failed" or
                    row.get("parent_supervisor_reason") != "worker exit 1" or
                    not row.get("resource_audit_sha256")):
                raise ValueError("failed board5 parent was falsely promoted")
            if origin == "roi_v2_2" and (row.get("parent_manifest_status") != "generated_unreviewed" or
                    row.get("parent_supervisor_status") != "generated_unreviewed" or
                    row.get("parent_supervisor_reason") is not None or
                    row.get("resource_audit_sha256") is not None):
                raise ValueError("ROI-v2 parent status differs")
    if origins != {"base43": 43, "board5_partial3": 3, "roi_v2_2": 2}:
        raise ValueError("composed candidate is not exactly 43+3+2")
    if (qa.get("schema") != "mustard_sam_composed_candidate_qa_v2" or
            qa.get("status") != "accepted" or
            qa.get("decision") != "accepted_for_coarse_pose_support" or
            qa.get("inventory_sha256") != inventory_sha256 or
            qa.get("contact_sheet_sha256") != inventory["contact_sheet_sha256"] or
            qa.get("reviewed_names") != list(TRAIN_NAMES) or
            not qa.get("reviewer") or not qa.get("reviewed_at")):
        raise ValueError("independent full-48 v2 candidate QA acceptance missing")
    audit = report.get("source_audit")
    if (not isinstance(audit, list) or len(audit) < 4 + 48 + 48 * 3 + 5 * 6 or
            any(not isinstance(pair, list) or len(pair) != 2 or
                not isinstance(pair[0], str) or not isinstance(pair[1], str) or
                len(pair[1]) != 64 or set(pair[1]) - set("0123456789abcdef") for pair in audit) or
            audit[1][1] != inventory["base_inventory_sha256"] or
            audit[2][1] != inventory["base_rejected_qa_sha256"] or
            audit[3][1] != inventory["selection_sha256"]):
        raise ValueError("composed v2 source audit missing metadata/masks")
    if (Path(audit[1][0]).name != "complete_inventory.json" or
            audit[0][1] != package_sha256 or
            [Path(audit[4 + index][0]) for index in range(48)] !=
            [Path(report["train_photo_dir"]) / name for name in TRAIN_NAMES] or
            [audit[4 + index][1] for index in range(48)] !=
            [report["train_photo_sha256"][name] for name in TRAIN_NAMES]):
        raise ValueError("composed v2 RGB source audit order differs")
    selection = sealed_json(Path(audit[3][0]), audit[3][1])
    if (selection.get("schema") != "sam21_mustard_candidate_selection_v2" or
            [r.get("name") for r in selection.get("images", [])] != list(candidate.CORRECTED)):
        raise ValueError("selected five changed after composition")
    original = sealed_json(Path(audit[1][0]), audit[1][1])
    original_rows = original.get("images", [])
    if [r.get("name") for r in original_rows] != list(TRAIN_NAMES):
        raise ValueError("original 48-view audit inventory order differs")
    selected_rows = {r["name"]: r for r in selection["images"]}
    cursor = 52
    base_root = Path(audit[1][0]).parent
    for row, source, old in zip(rows, report["images"], original_rows):
        name = row["name"]
        expected = [base_root / "frames" / name / "frame.json",
                    base_root / "frames" / name / "raw.png",
                    base_root / "frames" / name / "clean.png"]
        if audit[cursor][1] != old.get("frame_manifest_sha256") or audit[cursor + 2][1] != old.get("cleaned_mask_sha256"):
            raise ValueError("v2 original frame audit hashes differ")
        if name in selected_rows:
            chosen = selected_rows[name]
            parent = Path(chosen["parent_root"])
            expected.extend((parent / "manifest.json", parent / "supervisor.json",
                             Path(chosen["recipe_path"]), Path(chosen["review_path"])))
            if chosen.get("resource_audit_path") is not None:
                expected.append(Path(chosen["resource_audit_path"]))
            expected.append(parent / "review_sheet.jpg")
            expected.extend((parent / "raw_masks" / (name + ".png"),
                             parent / "cleaned_masks" / (name + ".png")))
            if source.get("source_clean_path") != str(expected[-1]):
                raise ValueError("v2 selected clean source path differs")
        else:
            if source.get("source_clean_path") != str(expected[-1]):
                raise ValueError("v2 base clean source path differs")
        if [Path(pair[0]) for pair in audit[cursor:cursor + len(expected)]] != expected:
            raise ValueError("v2 per-frame audit source path differs")
        cursor += len(expected)
    if cursor != len(audit):
        raise ValueError("v2 source audit has missing or extra entries")
    for source, expected in audit:
        path = Path(source)
        if path.is_symlink() or not path.is_file() or sha256(path) != expected:
            raise ValueError("composed v2 source or metadata changed after review")
    for source, expected_hash in audit:
        path = Path(source)
        if path.is_symlink() or not path.is_file() or sha256(path) != expected_hash:
            raise ValueError("composition source or metadata changed after review")


def preflight(package_path: Path, train_photos: Path, inventory_path: Path, inventory_root: Path,
              inventory_sha256: str, qa_path: Path, qa_sha256: str, *,
              output: Path = OUTPUT, mount: Path = MOUNT, require_mount: bool = True,
              package_sha256: str = PACKAGE_SHA256,
              expected_size: tuple[int, int] = (1280, 1024)) -> dict:
    if output.exists() or output.is_symlink() or output.parent.is_symlink():
        raise ValueError("staging output must be fresh")
    if require_mount:
        if (not mount.is_mount() or mount.stat().st_dev == ROOT.stat().st_dev or
                not output.resolve().is_relative_to(mount.resolve())):
            raise ValueError("staging destination must be external mounted volume")
    package = sealed_json(package_path, package_sha256)
    if (package.get("schema") != "ycb_object_evaluation_package_v1" or
            package.get("object_id") != "006_mustard_bottle" or
            package.get("manifest_sha256") != ACQUISITION_SHA256):
        raise ValueError("not the frozen mustard evaluation package")
    training = package.get("training_inputs", [])
    heldout = package.get("heldout_photos", [])
    train_names = [Path(row["path"]).name for row in training]
    heldout_names = [Path(row["path"]).name for row in heldout]
    if (train_names != list(TRAIN_NAMES) or heldout_names != list(HELDOUT_NAMES) or
            len(set(train_names + heldout_names)) != 60 or
            any(row["path"] != "photos/" + name for row, name in zip(training, TRAIN_NAMES))):
        raise ValueError("package is not exact 48-train/12-heldout split")
    exact_files(train_photos, set(TRAIN_NAMES))
    photo_hashes = {}
    for row, name in zip(training, TRAIN_NAMES):
        photo = train_photos / name
        if photo.stat().st_size != row["bytes"] or sha256(photo) != row["sha256"]:
            raise ValueError(f"train photo hash differs: {name}")
        photo_hashes[name] = row["sha256"]
    inventory = sealed_json(inventory_path, inventory_sha256)
    rows = inventory.get("images", [])
    kind = inventory.get("schema")
    if kind == "sam21_mustard_point_mask_inventory_v1":
        if (inventory.get("status") != "generated_unreviewed" or
                inventory.get("package_sha256") != package_sha256 or
                inventory.get("prompt_sha256") != PROMPTS_SHA256 or
                [row.get("name") for row in rows] != list(TRAIN_NAMES)):
            raise ValueError("full 48-view SAM inventory is incomplete or wrong source")
    elif kind not in ("sam21_mustard_m1_composed_candidate_v1", "sam21_mustard_m1_composed_candidate_v2"):
        raise ValueError("unknown full-48 mask inventory schema")
    qa = sealed_json(qa_path, qa_sha256)
    if kind == "sam21_mustard_point_mask_inventory_v1":
        if (qa.get("schema") != "mustard_sam_mask_qa_v1" or qa.get("status") != "accepted" or
                qa.get("decision") != "accepted_for_coarse_pose_support" or
                qa.get("inventory_sha256") != inventory_sha256 or
                qa.get("reviewed_names") != list(TRAIN_NAMES) or
                not qa.get("reviewer") or not qa.get("reviewed_at")):
            raise ValueError("full independent visual QA acceptance missing")
    elif kind == "sam21_mustard_m1_composed_candidate_v1":
        validate_composed_candidate(inventory, rows, qa, inventory_root, inventory_sha256, package_sha256)
    else:
        validate_composed_selection(inventory, rows, qa, inventory_root, inventory_sha256, package_sha256)
    qa_mask_hashes = qa.get("cleaned_mask_sha256")
    if not isinstance(qa_mask_hashes, dict) or set(qa_mask_hashes) != set(TRAIN_NAMES):
        raise ValueError("QA must bind all 48 cleaned mask hashes")
    frames = inventory_root / "frames"
    if frames.is_symlink() or not frames.is_dir() or {item.name for item in frames.iterdir()} != set(TRAIN_NAMES):
        raise ValueError("SAM frame directory is not exact 48-train set")
    masks = {}
    for row, name in zip(rows, TRAIN_NAMES):
        if row.get("source_sha256") != photo_hashes[name]:
            raise ValueError(f"SAM source photo differs: {name}")
        if row.get("cleaned_mask_path") != f"frames/{name}/clean.png":
            raise ValueError(f"SAM cleaned-mask relative path differs: {name}")
        path = frames / name / "clean.png"
        if (path.parent.is_symlink() or path.is_symlink() or not path.is_file() or
                row.get("cleaned_mask_sha256") != qa_mask_hashes[name] or
                sha256(path) != qa_mask_hashes[name]):
            raise ValueError(f"cleaned SAM mask hash/path differs: {name}")
        masks[name] = {"source": str(path), "sha256": qa_mask_hashes[name],
                       **mask_pixels(path, train_photos / name, expected_size)}
    names_bytes = ("\n".join(TRAIN_NAMES) + "\n").encode("ascii")
    total = sum((train_photos / name).stat().st_size for name in TRAIN_NAMES)
    total += sum(Path(row["source"]).stat().st_size for row in masks.values()) + len(names_bytes)
    if total > MAX_BYTES or shutil.disk_usage(mount).free < MIN_FREE + MAX_BYTES + BUFFER:
        raise ValueError("150 MiB stage cap or external 10 GiB reserve not met")
    if shutil.disk_usage(ROOT).free < MIN_FREE:
        raise ValueError("internal 10 GiB reserve not met")
    return {"schema": "mustard_train_only_stage_v1", "status": "preflight",
            "package_sha256": package_sha256, "acquisition_manifest_sha256": ACQUISITION_SHA256,
            "inventory_sha256": inventory_sha256, "qa_sha256": qa_sha256,
            "inventory_schema": kind,
            "train_photo_sha256": photo_hashes, "cleaned_masks": masks,
            "train_names_sha256": hashlib.sha256(names_bytes).hexdigest(),
            "train_names": list(TRAIN_NAMES), "heldout_names_excluded": list(HELDOUT_NAMES),
            "output": str(output), "copy_bytes": total,
            "mask_role": "accepted coarse pose support, not object silhouette or ground truth"}


def prepare(*args, output: Path = OUTPUT, mount: Path = MOUNT, **kwargs) -> dict:
    plan = preflight(*args, output=output, mount=mount, **kwargs)
    output.mkdir(parents=True)
    (output / "images").mkdir()
    (output / "masks").mkdir()
    started = time.monotonic()
    copied = 0
    try:
        for name in TRAIN_NAMES:
            for source, target, digest in (
                (Path(args[1]) / name, output / "images" / name, plan["train_photo_sha256"][name]),
                (Path(plan["cleaned_masks"][name]["source"]), output / "masks" / (name + ".png"),
                 plan["cleaned_masks"][name]["sha256"]),
            ):
                size = source.stat().st_size
                if (time.monotonic() - started > MAX_SECONDS or copied + size > MAX_BYTES or
                        shutil.disk_usage(mount).free < MIN_FREE + size + BUFFER or
                        shutil.disk_usage(ROOT).free < MIN_FREE):
                    raise ValueError("stage deadline, byte cap, or dual-disk reserve")
                shutil.copy2(source, target)
                copied += target.stat().st_size
                if sha256(source) != digest or sha256(target) != digest:
                    raise ValueError(f"stage copy/source hash differs: {name}")
        names = ("\n".join(TRAIN_NAMES) + "\n").encode("ascii")
        (output / "train-names.txt").write_bytes(names)
        copied += len(names)
        if hashlib.sha256(names).hexdigest() != plan["train_names_sha256"]:
            raise ValueError("train-name list changed")
        # Late-source and staged-copy rehash, plus no extra files.
        if ({p.name for p in (output / "images").iterdir()} != set(TRAIN_NAMES) or
                {p.name for p in (output / "masks").iterdir()} != {n + ".png" for n in TRAIN_NAMES}):
            raise ValueError("staged train file set changed")
        for name in TRAIN_NAMES:
            for source, target, digest in (
                (Path(args[1]) / name, output / "images" / name, plan["train_photo_sha256"][name]),
                (Path(plan["cleaned_masks"][name]["source"]), output / "masks" / (name + ".png"),
                 plan["cleaned_masks"][name]["sha256"]),
            ):
                if source.is_symlink() or target.is_symlink() or sha256(source) != digest or sha256(target) != digest:
                    raise ValueError(f"stage source/copy changed after copy: {name}")
        if (copied != plan["copy_bytes"] or copied > MAX_BYTES or
                time.monotonic() - started > MAX_SECONDS or
                shutil.disk_usage(mount).free < MIN_FREE or shutil.disk_usage(ROOT).free < MIN_FREE or
                sha256(Path(args[0])) != plan["package_sha256"] or
                sha256(Path(args[2])) != plan["inventory_sha256"] or
                sha256(Path(args[5])) != plan["qa_sha256"]):
            raise ValueError("stage postflight hashes, limits, or dual-disk reserve failed")
        plan.update(status="complete", copied_bytes=copied)
    except Exception as error:
        plan.update(status="failed", failure=f"{type(error).__name__}: {error}", copied_bytes=copied)
        (output / "stage-report.json").write_text(json.dumps(plan, indent=2, sort_keys=True) + "\n")
        raise
    (output / "stage-report.json").write_text(json.dumps(plan, indent=2, sort_keys=True) + "\n")
    return plan


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--train-photos", type=Path, required=True)
    parser.add_argument("--inventory", type=Path, required=True)
    parser.add_argument("--inventory-root", type=Path, required=True)
    parser.add_argument("--inventory-sha256", required=True)
    parser.add_argument("--qa", type=Path, required=True)
    parser.add_argument("--qa-sha256", required=True)
    parser.add_argument("--prepare", action="store_true", help="copy only after visual QA and root approval")
    args = parser.parse_args()
    params = (PACKAGE, args.train_photos, args.inventory, args.inventory_root,
              args.inventory_sha256, args.qa, args.qa_sha256)
    report = prepare(*params) if args.prepare else preflight(*params)
    print(json.dumps({key: report[key] for key in ("status", "output", "copy_bytes", "qa_sha256")}, indent=2))


if __name__ == "__main__":
    main()
