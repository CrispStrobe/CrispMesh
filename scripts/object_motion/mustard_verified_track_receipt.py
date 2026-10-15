"""Bounded TRAIN-only verified-track count receipt; no poses or track export.

The public command runs its read-only extractor in a supervised child and
writes only a fresh, small counts/hash JSON receipt after successful exit.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
from PIL import Image

from scripts.object_motion.verified_track_adapter import ImageSource, extract_candidate_tracks


ROOT = Path(__file__).resolve().parents[2]
DATA = Path("/Volumes/backups/code/crispmesh-data")
STAGE = DATA / "mustard-sfm-train-001"
PRODUCER = DATA / "mustard-sfm-masked-fixed-exhaustive-001"
STAGE_REPORT_SHA256 = "bbcba624f6a51f82e0ebdebf6e7e4cda10804bc8df34cfd0d5b3242983a225d0"
PRODUCER_RESULT_SHA256 = "5a83c3e05e2b66887411799ecc8c5f3f41e3b8af3bbd775d308055ef7ee0041e"
DATABASE_SHA256 = "5858be16dfa3f0fc50303d0eb4a001e21ee14036f67755a9997d5dac374b3075"
MIN_FREE = 10 * 1024**3
MAX_REPORT = 2 * 1024**2
MAX_METADATA = 2 * 1024**2
SUPERVISOR_SECONDS = 75
MAX_STDERR = 16 * 1024
MAX_ERROR_MESSAGE = 768


class ReceiptError(ValueError):
    pass


def _safe_failure(error: Exception) -> dict[str, str]:
    """Expose a bounded diagnostic, never a traceback or absolute path."""
    kind = re.sub(r"[^A-Za-z0-9_]", "", type(error).__name__)[:64] or "Error"
    message = str(error).replace("\n", " ").replace("\r", " ")
    path = re.search(r"(?:[A-Za-z]:[\\/]|/)", message)
    if path:
        message = message[:path.start()].rstrip(": ") + " <path>"
    message = re.sub(r"[^A-Za-z0-9 _.,:;=()<>-]", "?", message)[:MAX_ERROR_MESSAGE]
    return {"error_class": kind, "error_message": message or "unspecified failure"}


def _child_failure(stderr: bytes) -> str:
    if len(stderr) > MAX_STDERR:
        return "bounded diagnostic unavailable"
    try:
        failure = json.loads(stderr)
        kind, message = failure["error_class"], failure["error_message"]
        if (not isinstance(kind, str) or not isinstance(message, str) or
                len(kind) > 64 or len(message) > MAX_ERROR_MESSAGE or
                re.search(r"[^A-Za-z0-9_]", kind) or
                re.search(r"[^A-Za-z0-9 _.,:;=()<>-]", message)):
            raise ValueError("unsafe failure diagnostic")
        return f"{kind}: {message}"
    except (ValueError, KeyError, TypeError):
        return "bounded diagnostic unavailable"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def _is_sha(value) -> bool:
    return isinstance(value, str) and len(value) == 64 and all(c in "0123456789abcdef" for c in value)


def _bounded_json(path: Path, digest: str) -> dict:
    if (path.is_symlink() or not path.is_file() or not 0 < path.stat().st_size <= MAX_METADATA or
            sha256(path) != digest):
        raise ReceiptError(f"sealed metadata hash/size/path mismatch: {path}")
    value = json.loads(path.read_text())
    if not isinstance(value, dict):
        raise ReceiptError("sealed metadata is not an object")
    return value


def sources_from_reports(stage: dict, producer: dict, image_dir: Path) -> tuple[ImageSource, ...]:
    """Validate the producer's actual v1 report fields, not an inferred schema."""
    names = stage.get("train_names")
    photos = stage.get("train_photo_sha256")
    masks = stage.get("cleaned_masks")
    if (stage.get("schema") != "mustard_train_only_stage_v1" or stage.get("status") != "complete" or
            stage.get("mask_role") != "accepted coarse pose support, not object silhouette or ground truth" or
            not isinstance(names, list) or len(names) != 48 or len(set(names)) != 48 or
            not isinstance(photos, dict) or set(photos) != set(names) or
            not isinstance(masks, dict) or set(masks) != set(names) or
            stage.get("copied_bytes") != stage.get("copy_bytes") or
            not isinstance(stage.get("copy_bytes"), int) or stage["copy_bytes"] <= 0):
        raise ReceiptError("stage report not complete exact TRAIN v1")
    names_bytes = ("\n".join(names) + "\n").encode("ascii")
    if hashlib.sha256(names_bytes).hexdigest() != stage.get("train_names_sha256"):
        raise ReceiptError("TRAIN name-list digest differs")
    heldout = stage.get("heldout_names_excluded")
    if not isinstance(heldout, list) or set(names) & set(heldout):
        raise ReceiptError("held-out/train inventory overlap")
    sfm_source = producer.get("sfm_source")
    if (producer.get("schema") != "classical_backend_v1" or
            producer.get("status") != "sparse_complete" or
            not isinstance(sfm_source, dict) or
            sfm_source.get("kind") != "internal_image_only_pycolmap" or
            producer.get("sfm_intrinsics_policy") != "fixed-initial"):
        raise ReceiptError("producer report not sealed image-only fixed-initial SfM")
    entries = producer.get("inputs")
    pose_masks = producer.get("pose_masks")
    if (not isinstance(entries, list) or len(entries) != 48 or
            any(not isinstance(entry, dict) for entry in entries) or
            [entry.get("name") for entry in entries] != names or
            not isinstance(pose_masks, list) or len(pose_masks) != 48 or
            any(not isinstance(entry, dict) for entry in pose_masks) or
            [entry.get("name") for entry in pose_masks] != [name + ".png" for name in names]):
        raise ReceiptError("producer RGB/mask inventory differs from stage")
    output = []
    for name, entry, mask_entry in zip(names, entries, pose_masks):
        if (not isinstance(name, str) or Path(name).name != name or "\\" in name or
                not name.lower().endswith((".jpg", ".jpeg"))):
            raise ReceiptError("invalid exact TRAIN JPEG name")
        mask = masks[name]
        image_sha, mask_sha = photos[name], mask.get("sha256") if isinstance(mask, dict) else None
        if (not _is_sha(image_sha) or not _is_sha(mask_sha) or
                entry.get("sha256") != image_sha or mask_entry.get("sha256") != mask_sha):
            raise ReceiptError(f"producer/stage source hash or dimensions differ: {name}")
        photo = image_dir / name
        if photo.is_symlink() or not photo.is_file() or sha256(photo) != image_sha:
            raise ReceiptError(f"stage RGB bytes differ: {name}")
        with Image.open(photo) as image:
            if image.format != "JPEG" or image.mode != "RGB":
                raise ReceiptError(f"stage RGB format differs: {name}")
            width, height = image.size
        if (width < 2 or height < 2 or
                not isinstance(mask.get("kept_pixels"), int) or
                not isinstance(mask.get("ignored_pixels"), int) or
                mask["kept_pixels"] < 0 or mask["ignored_pixels"] < 0 or
                mask["kept_pixels"] + mask["ignored_pixels"] != width * height):
            raise ReceiptError(f"stage mask pixel counts/dimensions differ: {name}")
        output.append(ImageSource(name, image_sha, mask_sha, width, height))
    return tuple(output)


def _disk_floor(extra_output_bytes: int = 0) -> dict[str, int]:
    free = {"internal": shutil.disk_usage(ROOT).free, "external": shutil.disk_usage(DATA).free}
    if free["internal"] < MIN_FREE or free["external"] < MIN_FREE + extra_output_bytes:
        raise ReceiptError("both volumes require at least 10 GiB free")
    return free


def _worker_receipt() -> dict:
    _disk_floor()
    stage_path, producer_path = STAGE / "stage-report.json", PRODUCER / "result.json"
    stage = _bounded_json(stage_path, STAGE_REPORT_SHA256)
    producer = _bounded_json(producer_path, PRODUCER_RESULT_SHA256)
    sources = sources_from_reports(stage, producer, STAGE / "images")
    if stage.get("output") != str(STAGE):
        raise ReceiptError("stage output path differs from pinned source")
    if (not STAGE.is_dir() or STAGE.is_symlink() or
            not PRODUCER.is_dir() or PRODUCER.is_symlink()):
        raise ReceiptError("sealed source root missing or linked")
    result = extract_candidate_tracks(PRODUCER / "database.db", DATABASE_SHA256,
                                      STAGE / "images", STAGE / "masks", sources)
    if (sha256(stage_path) != STAGE_REPORT_SHA256 or
            sha256(producer_path) != PRODUCER_RESULT_SHA256):
        raise ReceiptError("sealed reports changed during extraction")
    _disk_floor()
    manifest = [{"name": s.name, "image_sha256": s.image_sha256,
                 "mask_sha256": s.mask_sha256, "width": s.width, "height": s.height}
                for s in sources]
    manifest_sha = hashlib.sha256(json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    return {
        "schema": "mustard_verified_track_receipt_v1",
        "status": "diagnostic_only",
        "scope": "sealed 48 TRAIN, verified-index edges, mask-supported candidates",
        "stage_report_sha256": STAGE_REPORT_SHA256,
        "producer_result_sha256": PRODUCER_RESULT_SHA256,
        "database_sha256": result.database_sha256,
        "input_manifest_sha256": manifest_sha,
        "runner_sha256": sha256(Path(__file__)),
        "adapter_sha256": sha256(Path(extract_candidate_tracks.__code__.co_filename)),
        "counts": {
            "images": result.image_count,
            "geometry_rows": result.geometry_rows,
            "empty_geometry_rows": result.empty_geometry_rows,
            "verified_edges": result.verified_edges,
            "mask_supported_edges": result.mask_supported_edges,
            "components": result.components,
            "candidate_tracks": len(result.tracks),
            "rejected_conflicting_components": result.rejected_conflicting_components,
            "rejected_oversize_components": result.rejected_oversize_components,
            "rejected_short_components": result.rejected_short_components,
        },
        "leakage_caution": "Coarse pose masks may contain rotating board/support; candidate tracks are not verified object-only geometry.",
        "no_pose_or_geometry_output": True,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--worker", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.worker:
        if args.output is not None:
            raise ReceiptError("worker cannot choose output")
        try:
            payload = (json.dumps(_worker_receipt(), sort_keys=True, separators=(",", ":")) + "\n").encode()
            if len(payload) > MAX_REPORT:
                raise ReceiptError("receipt exceeds 2 MiB")
            sys.stdout.buffer.write(payload)
        except Exception as error:
            sys.stderr.write(json.dumps(_safe_failure(error), separators=(",", ":")))
            raise SystemExit(2) from None
        return
    output = args.output
    if (output is None or output.parent != DATA or output.suffix != ".json" or
            output.exists() or output.is_symlink() or DATA.is_symlink() or not DATA.is_dir()):
        raise ReceiptError("output must be a fresh direct external JSON path")
    _disk_floor(MAX_REPORT)
    child = subprocess.run([sys.executable, "-m", "scripts.object_motion.mustard_verified_track_receipt", "--worker"],
                           cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                           timeout=SUPERVISOR_SECONDS, check=False)
    if child.returncode != 0:
        raise ReceiptError(f"supervised extractor failed: {_child_failure(child.stderr)}")
    if len(child.stdout) > MAX_REPORT or len(child.stderr) > MAX_STDERR:
        raise ReceiptError("supervised extractor exceeded output/log cap")
    receipt = json.loads(child.stdout)
    if receipt.get("status") != "diagnostic_only" or receipt.get("schema") != "mustard_verified_track_receipt_v1":
        raise ReceiptError("worker did not return exact diagnostic receipt")
    _disk_floor(MAX_REPORT)
    with output.open("xb") as stream:
        stream.write(child.stdout)


if __name__ == "__main__":
    main()
