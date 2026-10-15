"""Compose a new mustard mask candidate from 43 sealed base + 5 reviewed corrections.

This creates an unreviewed 48-view candidate, never an accepted SfM input.
The rejected base inventory and its QA decision remain immutable.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import time

from PIL import Image, ImageDraw

from scripts.classical_backend import mustard_stage as base
from scripts.object_motion import sam_point_trial as point

ROOT = Path(__file__).resolve().parents[2]
MOUNT = Path("/Volumes/backups")
BASE_ROOT = MOUNT / "code/crispmesh-data/sam21-mustard-m1-resume-001"
OUTPUT = MOUNT / "code/crispmesh-data/sam21-mustard-m1-candidate48-001"
OUTPUT_V2 = MOUNT / "code/crispmesh-data/sam21-mustard-m1-candidate48-002"
TRAIN48_PHOTOS = MOUNT / "code/crispmesh-data/sam21-m1-train48-001/photos"
REJECTED_QA = ROOT / "tests/datasets/sam21_mustard_m1_full_review.json"
BASE_SHA = "b47e8caec7d05fae7bb84ae9b3eec7d4a35b1457ca9df67a3b984cbaaeed8a98"
REJECTED_QA_SHA = "4084e8e458b55c566dd138f9d4b8b853ab2c68bfc20f5f831447a0cc99e9528a"
BOARD5_RUNNER_SHA = "57d6248e1c234a4ac1b5d00d2a8645594596e9a68ca104c3cad9ea61269a5c11"
BOARD5_PROMPTS_SHA = "8cc0b6fa49d977194870ef91f10b83a793294d6b3903cb1e08643016bdc9b4f8"
CORRECTED = tuple(f"NP3_{angle:03d}.jpg" for angle in (318, 330, 336, 342, 348))
MAX_BYTES = 20 * 1024**2
MIN_FREE = 10 * 1024**3
BUFFER = 64 * 1024**2
MAX_SECONDS = 180
BASE_BOX = [480, 300, 760, 650]
BOARD_POINT = [620, 330, 0]
BOARD5_PARTIAL_MANIFEST_SHA = "1008fc7bdbaa75ac33f17f8ddf44eaa275b3e98eb21466906060ebc3300c638c"
BOARD5_PARTIAL_SUPERVISOR_SHA = "b4485b72c539d59b6b3229917f0099ceb11359cdb8c9228bd0553208989619ce"
ROI_V2_RUNNER_SHA = "814fecab6cb30fafd3abd03d320ec1d4f3584437cc2697589168deb19a4da4d7"
ROI_V2_RECIPE_HELPER_SHA = "cacf2b031f3378bf8c2b9d64daf6c38ce17bdfe2e104b20008129eba82b33aaf"
PARTIAL_NAMES = ("NP3_318.jpg", "NP3_330.jpg", "NP3_348.jpg")
ROI_NAMES = ("NP3_336.jpg", "NP3_342.jpg")


def digest_object(value: object) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def mask_info(path: Path, expected_hash: str, size: tuple[int, int]) -> int:
    if path.is_symlink() or not path.is_file() or base.sha256(path) != expected_hash:
        raise ValueError(f"source mask hash/path differs: {path}")
    with Image.open(path) as image:
        image.load()
        if image.format != "PNG" or image.mode != "L" or image.size != size:
            raise ValueError(f"mask format/dimensions differ: {path}")
        counts = image.histogram()
        if sum(counts[1:255]) or counts[0] == 0 or counts[255] == 0:
            raise ValueError(f"mask is not binary with both classes: {path}")
        return counts[255]


def safe_relative(root: Path, relative: str, expected_prefix: str, name: str) -> Path:
    if not isinstance(relative, str):
        raise ValueError("missing corrected relative mask path")
    rel = Path(relative)
    if rel.is_absolute() or rel.parts != (expected_prefix, name + ".png"):
        raise ValueError("corrected mask path is not exact expected relative path")
    target = root / rel
    if root.is_symlink() or (root / expected_prefix).is_symlink() or target.is_symlink():
        raise ValueError("corrected mask path contains symlink")
    return target


def source_audit_hashes(audit: list) -> None:
    for source, expected in audit:
        path = Path(source)
        if path.is_symlink() or not path.is_file() or base.sha256(path) != expected:
            raise ValueError("base/correction source or metadata changed after composition")


def full_frame_contact_sheet(plan: dict, output: Path) -> str:
    """Show all 48 complete RGB frames beside their masks; no ROI/GT crop."""
    width, height, label = 128, 102, 20
    sheet = Image.new("RGB", (8 * 2 * width, 6 * (height + label)), "white")
    draw = ImageDraw.Draw(sheet)
    for index, row in enumerate(plan["images"]):
        name = row["name"]
        source_photo = Path(plan["train_photo_dir"]) / name
        source_mask = output / "frames" / name / "clean.png"
        with Image.open(source_photo) as photo, Image.open(source_mask) as alpha:
            photo.load(); alpha.load()
            if (photo.format != "JPEG" or photo.mode != "RGB" or
                    alpha.format != "PNG" or alpha.mode != "L" or
                    photo.size != alpha.size or
                    base.sha256(source_photo) != plan["train_photo_sha256"][name] or
                    base.sha256(source_mask) != row["cleaned_mask_sha256"]):
                raise ValueError("contact-sheet RGB/mask source differs")
            rgb = photo.resize((width, height), Image.Resampling.BILINEAR)
            binary = alpha.resize((width, height), Image.Resampling.NEAREST)
            overlay = Image.composite(rgb, Image.new("RGB", (width, height), (225, 50, 50)), binary)
            x, y = (index % 8) * (2 * width), (index // 8) * (height + label)
            sheet.paste(rgb, (x, y))
            sheet.paste(overlay, (x + width, y))
            draw.text((x + 2, y + height + 2), f"{name} {row['origin']}", fill="black")
    target = output / "full48-contact-sheet.png"
    sheet.save(target, format="PNG")
    return base.sha256(target)


def validate_selected_view(row: dict, original_record: dict, name: str,
                           expected_size: tuple[int, int]) -> tuple[dict, list]:
    """Bind one usable row to its actual parent status, prompt, QA and bytes."""
    origin = row["origin"]
    parent_root = Path(row["parent_root"])
    manifest_path, supervisor_path = parent_root / "manifest.json", parent_root / "supervisor.json"
    recipe_path, review_path = Path(row["recipe_path"]), Path(row["review_path"])
    metadata = ((manifest_path, row["parent_manifest_sha256"]),
                (supervisor_path, row["parent_supervisor_sha256"]),
                (recipe_path, row["recipe_sha256"]), (review_path, row["review_sha256"]))
    if (parent_root.is_symlink() or any(p.is_symlink() for p, _ in metadata) or
            any(not isinstance(h, str) or len(h) != 64 or set(h) - set("0123456789abcdef")
                for _, h in metadata)):
        raise ValueError("selected view has linked or unsealed parent metadata")
    manifest, supervisor, recipe, review = (base.sealed_json(p, h) for p, h in metadata)
    if (manifest.get("package_sha256") != base.PACKAGE_SHA256 or
            manifest.get("base_inventory_sha256") != BASE_SHA or
            manifest.get("rejected_full_qa_sha256") != REJECTED_QA_SHA or
            manifest.get("checkpoint_sha256") != point.MODEL_SHA or
            manifest.get("source_inventory_sha256") != point.SOURCE_SHA or
            review.get("schema") != "sam21_mask_artifact_subset_qa_v1" or
            review.get("status") != "accepted" or
            review.get("decision") != "accepted_for_coarse_pose_support" or
            review.get("parent_manifest_sha256") != row["parent_manifest_sha256"] or
            review.get("parent_supervisor_sha256") != row["parent_supervisor_sha256"] or
            review.get("parent_manifest_status") != manifest.get("status") or
            review.get("parent_supervisor_status") != supervisor.get("status") or
            review.get("parent_supervisor_reason") != supervisor.get("reason") or
            not review.get("reviewer") or not review.get("reviewed_at")):
        raise ValueError(f"selected view parent/model/QA lineage differs: {name}")
    parent_rows = manifest.get("images", [])
    expected_parent_names = list(CORRECTED if origin == "board5_partial3" else ROI_NAMES)
    if [r.get("name") for r in parent_rows] != expected_parent_names:
        raise ValueError("selected parent row set/order differs")
    selected_parent = next(r for r in parent_rows if r["name"] == name)
    if (selected_parent.get("status") != "complete_unreviewed" or
            selected_parent.get("source_sha256") != row["source_sha256"]):
        raise ValueError(f"selected parent row is incomplete or photo differs: {name}")
    if origin == "board5_partial3":
        if (row["parent_manifest_sha256"] != BOARD5_PARTIAL_MANIFEST_SHA or
                row["parent_supervisor_sha256"] != BOARD5_PARTIAL_SUPERVISOR_SHA or
                manifest.get("schema") != "sam21_mustard_m1_board5_candidate_v1" or
                manifest.get("status") != "partial_unusable" or
                manifest.get("runner_sha256") != BOARD5_RUNNER_SHA or
                manifest.get("correction_prompt_sha256") != BOARD5_PROMPTS_SHA or
                supervisor.get("schema") != "sam21_mustard_m1_board5_supervisor_v1" or
                supervisor.get("status") != "failed" or supervisor.get("reason") != "worker exit 1" or
                [r.get("status") for r in parent_rows] !=
                ["complete_unreviewed", "complete_unreviewed", "failed_unusable",
                 "failed_unusable", "complete_unreviewed"] or
                recipe.get("schema") != "sam21_mustard_m1_board_negative5_prompts_v1" or
                row["recipe_sha256"] != BOARD5_PROMPTS_SHA or
                recipe.get("original_prompt_sha256") != base.PROMPTS_SHA256 or
                recipe.get("base_inventory_sha256") != BASE_SHA or
                recipe.get("rejected_full_qa_sha256") != REJECTED_QA_SHA):
            raise ValueError("partial board5 parent cannot be promoted or relabelled")
        resource_path = Path(row["resource_audit_path"])
        resource_sha = row["resource_audit_sha256"]
        audit = base.sealed_json(resource_path, resource_sha)
        if (audit.get("schema") != "sam21_partial_parent_resource_audit_v1" or
                audit.get("status") != "accepted" or
                audit.get("parent_manifest_sha256") != row["parent_manifest_sha256"] or
                audit.get("parent_supervisor_sha256") != row["parent_supervisor_sha256"] or
                audit.get("parent_manifest_status") != "partial_unusable" or
                audit.get("parent_supervisor_status") != "failed" or
                audit.get("parent_supervisor_reason") != "worker exit 1" or
                audit.get("selected_names") != list(PARTIAL_NAMES) or
                audit.get("resource_caps_not_triggered") is not True or
                any(audit.get(k) != supervisor.get(k) for k in
                    ("seconds_total", "seconds_worker", "peak_worker_rss_kib")) or
                not (0 <= supervisor["seconds_total"] < 240 and
                     0 <= supervisor["seconds_worker"] < 90 and
                     0 <= supervisor["peak_worker_rss_kib"] < 1572864) or
                review.get("resource_audit_sha256") != resource_sha or
                not audit.get("reviewer") or not audit.get("reviewed_at")):
            raise ValueError("partial parent lacks accepted bound resource audit")
        extra_metadata = [(resource_path, resource_sha)]
        box = recipe.get("box_xyxy_original_pixels")
        recipe_rows = recipe.get("images", [])
        expected_points = original_record.get("points_xy_label", []) + [BOARD_POINT]
        if row.get("recipe_row_sha256") != digest_object({"name": name,
                "source_sha256": row["source_sha256"], "box_xyxy_original_pixels": box,
                "points_xy_label": expected_points}):
            raise ValueError("board5 per-view recipe hash differs")
    else:
        if (manifest.get("schema") != "sam21_mustard_m1_recipe_trial_v2" or
                manifest.get("status") != "generated_unreviewed" or
                manifest.get("recipe_sha256") != row["recipe_sha256"] or
                manifest.get("runner_sha256") != ROI_V2_RUNNER_SHA or
                base.sha256(ROOT / "scripts/object_motion/sam_m1_recipe_trial.py") != ROI_V2_RUNNER_SHA or
                manifest.get("recipe_helper_sha256") != ROI_V2_RECIPE_HELPER_SHA or
                base.sha256(ROOT / "scripts/object_motion/sam_prompt_recipe.py") != ROI_V2_RECIPE_HELPER_SHA or
                manifest.get("parent_board5_manifest_sha256") != BOARD5_PARTIAL_MANIFEST_SHA or
                manifest.get("parent_board5_supervisor_sha256") != BOARD5_PARTIAL_SUPERVISOR_SHA or
                supervisor.get("schema") != "sam21_mustard_m1_recipe_supervisor_v2" or
                supervisor.get("status") != "generated_unreviewed" or supervisor.get("reason") or
                recipe.get("schema") != "sam21_m1_prompt_recipe_v2" or
                recipe.get("package_sha256") != base.PACKAGE_SHA256 or
                recipe.get("original_prompt_sha256") != base.PROMPTS_SHA256 or
                recipe.get("base_inventory_sha256") != BASE_SHA or
                recipe.get("rejected_full_qa_sha256") != REJECTED_QA_SHA or
                recipe.get("checkpoint_sha256") != point.MODEL_SHA or
                recipe.get("source_inventory_sha256") != point.SOURCE_SHA or
                recipe.get("parent_board5_manifest_sha256") != BOARD5_PARTIAL_MANIFEST_SHA or
                recipe.get("parent_board5_supervisor_sha256") != BOARD5_PARTIAL_SUPERVISOR_SHA or
                row.get("resource_audit_path") is not None or
                row.get("resource_audit_sha256") is not None or
                review.get("resource_audit_sha256") is not None):
            raise ValueError("ROI-v2 parent/recipe lineage differs")
        extra_metadata = []
        box = recipe.get("box_xyxy_original_pixels")
        recipe_rows = recipe.get("images", [])
    expected_recipe_names = expected_parent_names
    if [r.get("name") for r in recipe_rows] != expected_recipe_names:
        raise ValueError("per-view recipe names/order differs")
    recipe_row = next(r for r in recipe_rows if r["name"] == name)
    points = recipe_row.get("points_xy_label")
    if (recipe_row.get("source_sha256") != row["source_sha256"] or
            selected_parent.get("points_xy_label") != points or
            selected_parent.get("box_xyxy_original_pixels", box) != box or
            row.get("recipe_row_sha256") != digest_object({"name": name,
                "source_sha256": row["source_sha256"], "box_xyxy_original_pixels": box,
                "points_xy_label": points}) or
            (origin == "roi_v2_2" and selected_parent.get("recipe_row_sha256") != row["recipe_row_sha256"])):
        raise ValueError("selected per-view prompt does not match recipe/parent")
    if (not isinstance(box, list) or len(box) != 4 or
            any(type(v) is not int for v in box) or
            not (0 <= box[0] < box[2] <= expected_size[0] and
                 0 <= box[1] < box[3] <= expected_size[1]) or
            not isinstance(points, list) or len(points) != 4 or
            any(not isinstance(p, list) or len(p) != 3 or
                any(type(v) is not int for v in p) or
                not (0 <= p[0] < expected_size[0] and 0 <= p[1] < expected_size[1]) or
                p[2] not in (0, 1) for p in points) or
            points[:3] != original_record.get("points_xy_label") or
            points[3][2] != 0 or
            not (box[0] <= points[0][0] < box[2] and box[1] <= points[0][1] < box[3])):
        raise ValueError("selected prompt pixels/box differ from supported image-coordinate recipe")
    names = list(PARTIAL_NAMES if origin == "board5_partial3" else ROI_NAMES)
    if (review.get("reviewed_names") != names or
            any(not isinstance(review.get(key), dict) or set(review[key]) != set(names) for key in
                ("source_image_sha256", "raw_mask_sha256", "cleaned_mask_sha256")) or
            review.get("source_image_sha256", {}).get(name) != row["source_sha256"] or
            review.get("raw_mask_sha256", {}).get(name) != selected_parent.get("raw_mask_sha256") or
            review.get("cleaned_mask_sha256", {}).get(name) != selected_parent.get("cleaned_mask_sha256") or
            not review.get("review_sheet_sha256")):
        raise ValueError("selected per-view root QA does not bind artifact or recipe")
    review_sheet = parent_root / "review_sheet.jpg"
    if (review_sheet.is_symlink() or not review_sheet.is_file() or
            base.sha256(review_sheet) != review["review_sheet_sha256"]):
        raise ValueError("selected parent visual-review sheet hash differs")
    with Image.open(review_sheet) as sheet:
        sheet.load()
        if sheet.format != "JPEG" or sheet.width == 0 or sheet.height == 0:
            raise ValueError("selected parent visual-review sheet is not JPEG")
    raw = safe_relative(parent_root, selected_parent.get("raw_mask_path"), "raw_masks", name)
    clean = safe_relative(parent_root, selected_parent.get("cleaned_mask_path"), "cleaned_masks", name)
    mask_info(raw, selected_parent["raw_mask_sha256"], expected_size)
    mask_info(clean, selected_parent["cleaned_mask_sha256"], expected_size)
    artifacts = [(str(p), h) for p, h in (*metadata, *extra_metadata)]
    artifacts.append((str(review_sheet), review["review_sheet_sha256"]))
    artifacts.extend(((str(raw), selected_parent["raw_mask_sha256"]),
                      (str(clean), selected_parent["cleaned_mask_sha256"])))
    replacement = {"name": name, "origin": origin, "source_sha256": row["source_sha256"],
        "source_clean_path": str(clean), "cleaned_mask_sha256": selected_parent["cleaned_mask_sha256"],
        "source_frame_manifest_sha256": row["parent_manifest_sha256"],
        "parent_supervisor_sha256": row["parent_supervisor_sha256"],
        "parent_manifest_status": manifest["status"], "parent_supervisor_status": supervisor["status"],
        "parent_supervisor_reason": supervisor.get("reason"),
        "prompt_sha256": row["recipe_sha256"], "prompt_row_sha256": row["recipe_row_sha256"],
        "per_view_qa_sha256": row["review_sha256"],
        "resource_audit_sha256": row.get("resource_audit_sha256")}
    return replacement, artifacts


def preflight_selection(selection_path: Path, selection_sha: str, *,
                        output: Path = OUTPUT_V2, base_root: Path = BASE_ROOT,
                        mount: Path = MOUNT, package_path: Path = base.PACKAGE,
                        train_photos: Path = TRAIN48_PHOTOS,
                        rejected_qa: Path = REJECTED_QA,
                        expected_size: tuple[int, int] = (1280, 1024),
                        require_mount: bool = True) -> dict:
    """Validate one explicit 43+3+2 selection without promoting a failed parent."""
    if (output.exists() or output.is_symlink() or output.parent.is_symlink() or
            not output.parent.is_dir() or base_root.is_symlink() or
            (base_root / "frames").is_symlink()):
        raise ValueError("candidate v2 output/base must be fresh real directories")
    if require_mount and (not mount.is_mount() or mount.stat().st_dev == ROOT.stat().st_dev or
                          output.parent.stat().st_dev != mount.stat().st_dev or
                          not output.resolve().is_relative_to(mount.resolve())):
        raise ValueError("candidate v2 output is not on mounted external volume")
    if (shutil.disk_usage(mount).free < MIN_FREE + MAX_BYTES + BUFFER or
            shutil.disk_usage(ROOT).free < MIN_FREE):
        raise ValueError("candidate v2 disk cap or dual-disk floor unavailable")
    package = base.sealed_json(package_path, base.PACKAGE_SHA256)
    training = package.get("training_inputs", [])
    if [row.get("path") for row in training] != ["photos/" + n for n in base.TRAIN_NAMES]:
        raise ValueError("candidate v2 package is not exact 48 TRAIN split")
    photo_hashes = {name: row["sha256"] for name, row in zip(base.TRAIN_NAMES, training)}
    if (train_photos.is_symlink() or not train_photos.is_dir() or
            {p.name for p in train_photos.iterdir()} != set(base.TRAIN_NAMES) or
            any(p.is_symlink() or not p.is_file() for p in train_photos.iterdir())):
        raise ValueError("candidate v2 photo directory is not exact 48 TRAIN set")
    for name, package_row in zip(base.TRAIN_NAMES, training):
        path = train_photos / name
        if path.stat().st_size != package_row["bytes"] or base.sha256(path) != photo_hashes[name]:
            raise ValueError(f"candidate v2 original photo differs: {name}")
    inventory_path = base_root / "complete_inventory.json"
    original = base.sealed_json(inventory_path, BASE_SHA)
    old_rows = original.get("images", [])
    rejected = base.sealed_json(rejected_qa, REJECTED_QA_SHA)
    if (original.get("schema") != "sam21_mustard_point_mask_inventory_v1" or
            original.get("status") != "generated_unreviewed" or
            original.get("package_sha256") != base.PACKAGE_SHA256 or
            original.get("prompt_sha256") != base.PROMPTS_SHA256 or
            [r.get("name") for r in old_rows] != list(base.TRAIN_NAMES) or
            rejected.get("status") != "rejected" or
            rejected.get("inventory_sha256") != BASE_SHA or
            rejected.get("rejected_names") != list(CORRECTED) or
            rejected.get("reviewed_names") != list(base.TRAIN_NAMES) or
            rejected.get("approved_for_sfm") is not False):
        raise ValueError("candidate v2 original/rejected lineage differs")
    selection = base.sealed_json(selection_path, selection_sha)
    replacement_rows = selection.get("images", [])
    if (selection.get("schema") != "sam21_mustard_candidate_selection_v2" or
            selection.get("status") != "reviewed_per_view_not_full48" or
            selection.get("package_sha256") != base.PACKAGE_SHA256 or
            selection.get("base_inventory_sha256") != BASE_SHA or
            selection.get("rejected_full_qa_sha256") != REJECTED_QA_SHA or
            [r.get("name") for r in replacement_rows] != list(CORRECTED)):
        raise ValueError("candidate v2 selection is not exact five reviewed TRAIN views")
    source_audit = [(str(p), h) for p, h in ((package_path, base.PACKAGE_SHA256),
        (inventory_path, BASE_SHA), (rejected_qa, REJECTED_QA_SHA),
        (selection_path, selection_sha))]
    source_audit.extend((str(train_photos / name), photo_hashes[name]) for name in base.TRAIN_NAMES)
    chosen = {r["name"]: r for r in replacement_rows}
    selected = []
    if ({p.name for p in (base_root / "frames").iterdir()} != set(base.TRAIN_NAMES) or
            any(p.is_symlink() or not p.is_dir() for p in (base_root / "frames").iterdir())):
        raise ValueError("candidate v2 original frame set differs")
    for old, name in zip(old_rows, base.TRAIN_NAMES):
        frame = base_root / "frames" / name
        if frame.is_symlink() or {p.name for p in frame.iterdir()} != {"frame.json", "raw.png", "clean.png"}:
            raise ValueError(f"candidate v2 base frame set differs: {name}")
        record_path = frame / "frame.json"
        record = base.sealed_json(record_path, old["frame_manifest_sha256"])
        if (old.get("source_sha256") != photo_hashes[name] or
                old.get("cleaned_mask_path") != f"frames/{name}/clean.png" or
                record.get("name") != name or record.get("source_sha256") != photo_hashes[name] or
                record.get("prompt_sha256") != base.PROMPTS_SHA256 or
                record.get("cleaned_mask_sha256") != old.get("cleaned_mask_sha256") or
                rejected.get("cleaned_mask_sha256", {}).get(name) != old.get("cleaned_mask_sha256")):
            raise ValueError(f"candidate v2 original frame/photo/prompt differs: {name}")
        raw = frame / "raw.png"
        clean = frame / "clean.png"
        mask_info(raw, record["raw_mask_sha256"], expected_size)
        mask_info(clean, old["cleaned_mask_sha256"], expected_size)
        source_audit.extend(((str(record_path), old["frame_manifest_sha256"]),
                             (str(raw), record["raw_mask_sha256"]),
                             (str(clean), old["cleaned_mask_sha256"])))
        if name in chosen:
            row = chosen[name]
            origin = "board5_partial3" if name in PARTIAL_NAMES else "roi_v2_2"
            if row.get("origin") != origin or row.get("source_sha256") != photo_hashes[name]:
                raise ValueError(f"candidate v2 origin/photo differs: {name}")
            replacement, artifacts = validate_selected_view(row, record, name, expected_size)
            source_audit.extend(artifacts)
            selected.append(replacement)
        else:
            selected.append({"name": name, "origin": "base43", "source_sha256": photo_hashes[name],
                "source_clean_path": str(clean), "cleaned_mask_sha256": old["cleaned_mask_sha256"],
                "source_frame_manifest_sha256": old["frame_manifest_sha256"],
                "prompt_sha256": base.PROMPTS_SHA256,
                "prompt_row_sha256": digest_object({"name": name,
                    "source_sha256": photo_hashes[name],
                    "box_xyxy_original_pixels": BASE_BOX,
                    "points_xy_label": record["points_xy_label"]})})
    if (sum(r["origin"] == "base43" for r in selected) != 43 or
            sum(r["origin"] == "board5_partial3" for r in selected) != 3 or
            sum(r["origin"] == "roi_v2_2" for r in selected) != 2):
        raise ValueError("candidate v2 must be 43+3+2 exactly")
    source_audit_hashes(source_audit)
    return {"schema": "sam21_mustard_m1_composition_report_v2", "status": "preflight",
            "base_inventory_sha256": BASE_SHA, "base_rejected_qa_sha256": REJECTED_QA_SHA,
            "selection_sha256": selection_sha, "package_sha256": base.PACKAGE_SHA256,
            "runner_sha256": base.sha256(Path(__file__)), "images": selected,
            "train_photo_dir": str(train_photos), "train_photo_sha256": photo_hashes,
            "source_audit": source_audit, "output": str(output),
            "parent_status_disclosure": {
                "board5_partial3_manifest_status": "partial_unusable",
                "board5_partial3_supervisor_status": "failed",
                "board5_partial3_supervisor_reason": "worker exit 1"},
            "mask_role": "coarse pose support, not exact silhouette"}


def preflight(base_root: Path, correction_root: Path, correction_manifest_sha: str,
              correction_prompts: Path, correction_prompts_sha: str,
              correction_qa: Path, correction_qa_sha: str, *,
              output: Path = OUTPUT, mount: Path = MOUNT,
              package_path: Path = base.PACKAGE, rejected_qa: Path = REJECTED_QA,
              expected_size: tuple[int, int] = (1280, 1024),
              require_mount: bool = True) -> dict:
    if (base_root.is_symlink() or correction_root.is_symlink() or
            (base_root / "frames").is_symlink() or
            output.exists() or output.is_symlink() or output.parent.is_symlink() or
            not output.parent.is_dir()):
        raise ValueError("candidate output must be a fresh real external directory")
    if require_mount and (not mount.is_mount() or mount.stat().st_dev == ROOT.stat().st_dev or
                          output.parent.stat().st_dev != mount.stat().st_dev or
                          not output.resolve().is_relative_to(mount.resolve())):
        raise ValueError("candidate output is not on a distinct mounted external volume")
    if (shutil.disk_usage(mount).free < MIN_FREE + MAX_BYTES + BUFFER or
            shutil.disk_usage(ROOT).free < MIN_FREE):
        raise ValueError("candidate disk cap or dual-disk floor unavailable")
    package = base.sealed_json(package_path, base.PACKAGE_SHA256)
    training = package.get("training_inputs", [])
    if [row.get("path") for row in training] != ["photos/" + n for n in base.TRAIN_NAMES]:
        raise ValueError("candidate source is not exact 48 TRAIN split")
    photo_hashes = {name: row["sha256"] for name, row in zip(base.TRAIN_NAMES, training)}
    base_inventory = base.sealed_json(base_root / "complete_inventory.json", BASE_SHA)
    frames_root = base_root / "frames"
    if ({item.name for item in frames_root.iterdir()} != set(base.TRAIN_NAMES) or
            any(item.is_symlink() or not item.is_dir() for item in frames_root.iterdir())):
        raise ValueError("base frames are not exactly 48 real directories")
    old_rows = base_inventory.get("images", [])
    if (base_inventory.get("schema") != "sam21_mustard_point_mask_inventory_v1" or
            base_inventory.get("status") != "generated_unreviewed" or
            base_inventory.get("package_sha256") != base.PACKAGE_SHA256 or
            base_inventory.get("prompt_sha256") != base.PROMPTS_SHA256 or
            [row.get("name") for row in old_rows] != list(base.TRAIN_NAMES)):
        raise ValueError("base 48 inventory seal or lineage differs")
    rejection = base.sealed_json(rejected_qa, REJECTED_QA_SHA)
    if (rejection.get("status") != "rejected" or rejection.get("inventory_sha256") != BASE_SHA or
            rejection.get("rejected_names") != list(CORRECTED) or
            rejection.get("reviewed_names") != list(base.TRAIN_NAMES) or
            rejection.get("approved_for_sfm") is not False):
        raise ValueError("base full-set rejection does not bind exact five failed views")
    correction = base.sealed_json(correction_root / "manifest.json", correction_manifest_sha)
    prompts = base.sealed_json(correction_prompts, correction_prompts_sha)
    corrected_rows = correction.get("images", [])
    prompt_rows = prompts.get("images", [])
    if (correction.get("schema") != "sam21_mustard_m1_board5_candidate_v1" or
            correction.get("status") != "generated_unreviewed" or
            correction.get("package_sha256") != base.PACKAGE_SHA256 or
            correction.get("original_prompt_sha256") != base.PROMPTS_SHA256 or
            correction.get("correction_prompt_sha256") != correction_prompts_sha or
            correction_prompts_sha != BOARD5_PROMPTS_SHA or
            correction_prompts_sha == base.PROMPTS_SHA256 or
            correction.get("base_inventory_sha256") != BASE_SHA or
            correction.get("rejected_full_qa_sha256") != REJECTED_QA_SHA or
            correction.get("checkpoint_sha256") != point.MODEL_SHA or
            correction.get("source_inventory_sha256") != point.SOURCE_SHA or
            correction.get("runner_sha256") != BOARD5_RUNNER_SHA or
            base.sha256(ROOT / "scripts/object_motion/sam_m1_board5.py") != BOARD5_RUNNER_SHA or
            prompts.get("schema") != "sam21_mustard_m1_board_negative5_prompts_v1" or
            prompts.get("original_prompt_sha256") != base.PROMPTS_SHA256 or
            prompts.get("base_inventory_sha256") != BASE_SHA or
            prompts.get("rejected_full_qa_sha256") != REJECTED_QA_SHA or
            [row.get("name") for row in corrected_rows] != list(CORRECTED) or
            [row.get("name") for row in prompt_rows] != list(CORRECTED)):
        raise ValueError("corrected five-view manifest/prompt lineage differs")
    supervisor_path = correction_root / "supervisor.json"
    if supervisor_path.is_symlink() or not supervisor_path.is_file():
        raise ValueError("corrected five-view supervisor evidence missing")
    supervisor_sha = base.sha256(supervisor_path)
    supervisor = base.sealed_json(supervisor_path, supervisor_sha)
    if supervisor.get("schema") != "sam21_mustard_m1_board5_supervisor_v1" or supervisor.get("status") != "generated_unreviewed" or supervisor.get("reason"):
        raise ValueError("corrected five-view supervisor did not finish successfully")
    for directory, expected in (("raw_masks", {n + ".png" for n in CORRECTED}),
                                ("cleaned_masks", {n + ".png" for n in CORRECTED})):
        folder = correction_root / directory
        if (folder.is_symlink() or not folder.is_dir() or
                {item.name for item in folder.iterdir()} != expected or
                any(item.is_symlink() or not item.is_file() for item in folder.iterdir())):
            raise ValueError("corrected mask directory contains extra, missing, or linked files")
    correction_review = base.sealed_json(correction_qa, correction_qa_sha)
    if (correction_review.get("schema") != "sam21_mustard_m1_board5_review_v1" or
            correction_review.get("status") != "accepted" or
            correction_review.get("decision") != "accepted_for_coarse_pose_support" or
            correction_review.get("manifest_sha256") != correction_manifest_sha or
            correction_review.get("correction_prompt_sha256") != correction_prompts_sha or
            correction_review.get("reviewed_names") != list(CORRECTED) or
            not correction_review.get("reviewer") or not correction_review.get("reviewed_at")):
        raise ValueError("independent corrected-five visual QA acceptance missing")
    accepted_hashes = correction_review.get("cleaned_mask_sha256")
    if not isinstance(accepted_hashes, dict) or set(accepted_hashes) != set(CORRECTED):
        raise ValueError("corrected-five QA must bind all cleaned mask hashes")
    selected = []
    source_audit = [(str(path), hash_value) for path, hash_value in (
        (package_path, base.PACKAGE_SHA256), (base_root / "complete_inventory.json", BASE_SHA),
        (rejected_qa, REJECTED_QA_SHA), (correction_root / "manifest.json", correction_manifest_sha),
        (correction_prompts, correction_prompts_sha), (correction_qa, correction_qa_sha),
        (supervisor_path, supervisor_sha))]
    for old, name in zip(old_rows, base.TRAIN_NAMES):
        frame = base_root / "frames" / name
        if frame.is_symlink() or {item.name for item in frame.iterdir()} != {"frame.json", "raw.png", "clean.png"}:
            raise ValueError(f"base frame has extra, missing, or linked entries: {name}")
        record_path = frame / "frame.json"
        record = base.sealed_json(record_path, old["frame_manifest_sha256"])
        if (old.get("source_sha256") != photo_hashes[name] or
                record.get("name") != name or record.get("source_sha256") != photo_hashes[name] or
                record.get("prompt_sha256") != base.PROMPTS_SHA256 or
                record.get("cleaned_mask_sha256") != old.get("cleaned_mask_sha256") or
                rejection["cleaned_mask_sha256"].get(name) != old.get("cleaned_mask_sha256")):
            raise ValueError(f"base frame/photo/prompt/hash differs: {name}")
        raw = frame / "raw.png"
        clean = frame / "clean.png"
        mask_info(raw, record["raw_mask_sha256"], expected_size)
        mask_info(clean, old["cleaned_mask_sha256"], expected_size)
        source_audit.extend(((str(record_path), old["frame_manifest_sha256"]),
                             (str(raw), record["raw_mask_sha256"]),
                             (str(clean), old["cleaned_mask_sha256"])))
        if name in CORRECTED:
            index = CORRECTED.index(name)
            row, prompt = corrected_rows[index], prompt_rows[index]
            expected_points = record.get("points_xy_label", []) + [[620, 330, 0]]
            if (row.get("source_sha256") != photo_hashes[name] or
                    prompt.get("source_sha256") != photo_hashes[name] or
                    row.get("points_xy_label") != prompt.get("points_xy_label") or
                    row.get("points_xy_label") != expected_points or
                    row.get("cleaned_mask_sha256") != accepted_hashes[name]):
                raise ValueError(f"corrected frame photo/prompt/QA differs: {name}")
            raw = safe_relative(correction_root, row.get("raw_mask_path"), "raw_masks", name)
            clean = safe_relative(correction_root, row.get("cleaned_mask_path"), "cleaned_masks", name)
            mask_info(raw, row["raw_mask_sha256"], expected_size)
            mask_info(clean, row["cleaned_mask_sha256"], expected_size)
            source_audit.extend(((str(raw), row["raw_mask_sha256"]),
                                 (str(clean), row["cleaned_mask_sha256"])))
            origin = "board5"
            frame_sha = correction_manifest_sha
            prompt_sha = correction_prompts_sha
            points = row["points_xy_label"]
            cleaned_sha = row["cleaned_mask_sha256"]
        else:
            origin = "base43"
            frame_sha = old["frame_manifest_sha256"]
            prompt_sha = base.PROMPTS_SHA256
            points = record["points_xy_label"]
            cleaned_sha = old["cleaned_mask_sha256"]
        selected.append({"name": name, "source_sha256": photo_hashes[name],
                         "source_clean_path": str(clean), "cleaned_mask_sha256": cleaned_sha,
                         "origin": origin, "source_frame_manifest_sha256": frame_sha,
                         "prompt_sha256": prompt_sha, "prompt_points_sha256": digest_object(points)})
    if [row["origin"] for row in selected].count("base43") != 43:
        raise ValueError("candidate must preserve exactly 43 original masks")
    return {"schema": "sam21_mustard_m1_composition_report_v1", "status": "preflight",
            "base_inventory_sha256": BASE_SHA, "base_rejected_qa_sha256": REJECTED_QA_SHA,
            "correction_manifest_sha256": correction_manifest_sha,
            "correction_prompt_sha256": correction_prompts_sha,
            "correction_qa_sha256": correction_qa_sha,
            "correction_supervisor_sha256": supervisor_sha,
            "package_sha256": base.PACKAGE_SHA256, "runner_sha256": base.sha256(Path(__file__)),
            "images": selected, "source_audit": source_audit, "output": str(output),
            "mask_role": "coarse pose support, not exact silhouette"}


def compose_plan(plan: dict, output: Path) -> dict:
    output.mkdir()
    (output / "frames").mkdir()
    started = time.monotonic()
    copied = 0
    try:
        for row in plan["images"]:
            source = Path(row["source_clean_path"])
            target_dir = output / "frames" / row["name"]
            target_dir.mkdir()
            target = target_dir / "clean.png"
            size = source.stat().st_size
            if (time.monotonic() - started > MAX_SECONDS or copied + size > MAX_BYTES or
                    shutil.disk_usage(output).free < MIN_FREE + size + BUFFER or
                    shutil.disk_usage(ROOT).free < MIN_FREE):
                raise ValueError("composition deadline, cap, or dual-disk floor")
            shutil.copy2(source, target)
            copied += target.stat().st_size
            if base.sha256(source) != row["cleaned_mask_sha256"] or base.sha256(target) != row["cleaned_mask_sha256"]:
                raise ValueError("source/copy hash changed during composition")
        for source, expected in plan["source_audit"]:
            path = Path(source)
            if path.is_symlink() or not path.is_file() or base.sha256(path) != expected:
                raise ValueError("base/correction source or metadata changed after composition")
        for row in plan["images"]:
            if (base.sha256(Path(row["source_clean_path"])) != row["cleaned_mask_sha256"] or
                    base.sha256(output / "frames" / row["name"] / "clean.png") != row["cleaned_mask_sha256"]):
                raise ValueError("source/copy hash changed after composition")
        if (time.monotonic() - started > MAX_SECONDS or copied > MAX_BYTES or
                shutil.disk_usage(output).free < MIN_FREE or shutil.disk_usage(ROOT).free < MIN_FREE or
                base.sha256(Path(__file__)) != plan["runner_sha256"]):
            raise ValueError("composition postflight resource or runner seal failed")
        if plan["schema"] == "sam21_mustard_m1_composition_report_v2":
            plan["contact_sheet_sha256"] = full_frame_contact_sheet(plan, output)
            plan["contact_sheet_path"] = "full48-contact-sheet.png"
            source_audit_hashes(plan["source_audit"])
        plan["status"] = "complete_unreviewed"
        plan["copied_mask_bytes"] = copied
        report_path = output / "composition-report.json"
        report_path.write_text(json.dumps(plan, indent=2, sort_keys=True) + "\n")
        v2 = plan["schema"] == "sam21_mustard_m1_composition_report_v2"
        inventory = {"schema": ("sam21_mustard_m1_composed_candidate_v2" if v2 else
                                "sam21_mustard_m1_composed_candidate_v1"),
                     "status": "generated_unreviewed", "full_training_package_ready": False,
                     "package_sha256": base.PACKAGE_SHA256, "base_inventory_sha256": BASE_SHA,
                     "base_rejected_qa_sha256": REJECTED_QA_SHA,
                     "composition_report_sha256": base.sha256(report_path),
                     "images": [{"name": row["name"], "source_sha256": row["source_sha256"],
                                 "cleaned_mask_path": f"frames/{row['name']}/clean.png",
                                 "cleaned_mask_sha256": row["cleaned_mask_sha256"],
                                 "origin": row["origin"],
                                 "source_frame_manifest_sha256": row["source_frame_manifest_sha256"],
                                 "prompt_sha256": row["prompt_sha256"],
                                 **({"prompt_row_sha256": row["prompt_row_sha256"],
                                     "parent_supervisor_sha256": row.get("parent_supervisor_sha256"),
                                     "parent_manifest_status": row.get("parent_manifest_status"),
                                     "parent_supervisor_status": row.get("parent_supervisor_status"),
                                     "parent_supervisor_reason": row.get("parent_supervisor_reason"),
                                     "per_view_qa_sha256": row.get("per_view_qa_sha256"),
                                     "resource_audit_sha256": row.get("resource_audit_sha256")}
                                    if v2 else {"prompt_points_sha256": row["prompt_points_sha256"]})}
                                for row in plan["images"]]}
        if v2:
            inventory["selection_sha256"] = plan["selection_sha256"]
            inventory["parent_status_disclosure"] = plan["parent_status_disclosure"]
            inventory["contact_sheet_sha256"] = plan["contact_sheet_sha256"]
            inventory["contact_sheet_path"] = plan["contact_sheet_path"]
        else:
            for key in ("correction_manifest_sha256", "correction_prompt_sha256",
                        "correction_qa_sha256", "correction_supervisor_sha256"):
                inventory[key] = plan[key]
        (output / "complete_inventory.json").write_text(json.dumps(inventory, indent=2, sort_keys=True) + "\n")
        output_items = list(output.rglob("*"))
        if any(item.is_symlink() for item in output_items):
            raise ValueError("composition output contains linked entry")
        output_bytes = sum(item.stat().st_size for item in output_items if item.is_file())
        if (output_bytes > MAX_BYTES or time.monotonic() - started > MAX_SECONDS or
                shutil.disk_usage(output).free < MIN_FREE or shutil.disk_usage(ROOT).free < MIN_FREE):
            raise ValueError("composition total output, deadline, or dual-disk postflight cap")
        if v2:
            source_audit_hashes(plan["source_audit"])
    except BaseException as error:
        plan.update(status="failed", failure=f"{type(error).__name__}: {error}", copied_mask_bytes=copied)
        (output / "composition-report.json").write_text(json.dumps(plan, indent=2, sort_keys=True) + "\n")
        raise
    return inventory


def compose(*args, output: Path = OUTPUT, **kwargs) -> dict:
    return compose_plan(preflight(*args, output=output, **kwargs), output)


def compose_selection(selection_path: Path, selection_sha: str, *,
                      output: Path = OUTPUT_V2, **kwargs) -> dict:
    return compose_plan(preflight_selection(selection_path, selection_sha,
                                          output=output, **kwargs), output)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--correction-root", type=Path)
    parser.add_argument("--correction-manifest-sha256")
    parser.add_argument("--correction-prompts", type=Path)
    parser.add_argument("--correction-prompts-sha256")
    parser.add_argument("--correction-qa", type=Path)
    parser.add_argument("--correction-qa-sha256")
    parser.add_argument("--selection", type=Path, help="sealed per-view 43+3+2 selector")
    parser.add_argument("--selection-sha256")
    parser.add_argument("--compose", action="store_true", help="copy only after independent board5 QA")
    args = parser.parse_args()
    if args.selection is not None:
        if (not args.selection_sha256 or any(getattr(args, name) is not None for name in
               ("correction_root", "correction_manifest_sha256", "correction_prompts",
                "correction_prompts_sha256", "correction_qa", "correction_qa_sha256"))):
            parser.error("selection v2 cannot mix with whole-board5 v1 arguments")
        result = (compose_selection(args.selection, args.selection_sha256) if args.compose else
                  preflight_selection(args.selection, args.selection_sha256))
    else:
        params = (args.correction_root, args.correction_manifest_sha256,
                  args.correction_prompts, args.correction_prompts_sha256,
                  args.correction_qa, args.correction_qa_sha256)
        if any(value is None for value in params):
            parser.error("whole-board5 v1 requires all correction arguments")
        result = (compose(BASE_ROOT, *params) if args.compose else preflight(BASE_ROOT, *params))
    print(json.dumps({key: result[key] for key in ("schema", "status", "package_sha256")}, indent=2))


if __name__ == "__main__":
    main()
