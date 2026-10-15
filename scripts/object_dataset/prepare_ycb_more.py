#!/usr/bin/env python3
"""Bounded preflight/fetch for two additional YCB object/photo-scan pairs.

Large temporary archives stay on /mnt/storage. This imports only the already
reviewed prepare_ycb.py tar selector and geometry-preserving scan converter.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import tempfile
import time
from urllib.request import Request, urlopen

from scripts.object_dataset import prepare_ycb as base


BASE_URL = "https://ycb-benchmarks.s3.amazonaws.com/data/"
STORAGE_ROOT = Path("/mnt/akademie_storage/crispmesh-data/ycb-expansion-001")
STORAGE_FLOOR = 10 * 1024**3
MAX_EXTRACT = 200_000_000
MAX_SECONDS = 900
MAX_MEMBERS = 2500
PHOTO_SIZE = (1280, 1024)
OBJECTS = {
    "006_mustard_bottle": {
        "berkeley_rgbd": ("berkeley/006_mustard_bottle/006_mustard_bottle_berkeley_rgbd.tgz",
                          657_272_400, '"c15b25428f8eea3432996eb2b4565de3-79"'),
        "google": ("google/006_mustard_bottle_google_16k.tgz",
                   10_703_139, '"36d5d7cea8336172a8cdc34b253459d8-2"'),
        "scan_resolution": "16k",
    },
    "035_power_drill": {
        "berkeley_rgbd": ("berkeley/035_power_drill/035_power_drill_berkeley_rgbd.tgz",
                          631_773_983, '"c9e9201a2de0ad6c1b075feebeaedffb-76"'),
        "google": ("google/035_power_drill_google_64k.tgz",
                   12_777_450, '"b6802a421551a54684887a809b28f5ec-2"'),
        "scan_resolution": "64k",
    },
}


class AcquisitionError(RuntimeError):
    def __init__(self, phase, original):
        self.phase = phase
        super().__init__(f"{phase}: {type(original).__name__}: {original}")


def sha256_file(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def source_info(object_id, role):
    if object_id not in OBJECTS or role not in ("berkeley_rgbd", "google"):
        raise ValueError("unsupported object or source role")
    relative, size, etag = OBJECTS[object_id][role]
    return {"url": BASE_URL + relative, "filename": Path(relative).name,
            "declared_bytes": size, "declared_etag": etag}


def _response_metadata(response):
    try:
        size = int(response.headers.get("Content-Length", ""))
    except ValueError as error:
        raise ValueError("missing or invalid Content-Length") from error
    return int(response.status), size, response.headers.get("ETag")


def preflight(object_id):
    """HEAD only: source availability/declared size, no transferred object body."""
    records = {}
    for role in ("berkeley_rgbd", "google"):
        info = source_info(object_id, role)
        with urlopen(Request(info["url"], method="HEAD"), timeout=30) as response:
            status, size, etag = _response_metadata(response)
        if (status, size, etag) != (200, info["declared_bytes"], info["declared_etag"]):
            raise ValueError(f"upstream HEAD metadata drift for {object_id}/{role}")
        records[role] = {**info, "head_status": status,
                         "head_content_length": size, "head_etag": etag,
                         "etag_is_cryptographic_hash": False}
    return {"schema": "ycb_more_head_preflight_v1", "object_id": object_id,
            "head_only": True, "sources": records,
            "caveat": "ETag and Content-Length are drift checks, not independent SHA-256 authentication"}


def _reserve(root, extra=0):
    if shutil.disk_usage(root).free < STORAGE_FLOOR + extra:
        raise OSError("storage acquisition would violate 10 GiB reserve")


def _download(stage, info, expected_sha256, deadline):
    """GET to a fresh storage-side file; retain observed digest, optionally pin it."""
    target = stage / "archives" / info["filename"]
    target.parent.mkdir(parents=True, exist_ok=True)
    digest = hashlib.sha256()
    count = 0
    try:
        response = urlopen(info["url"], timeout=30)
    except OSError as error:
        raise AcquisitionError("open_https_get", error) from error
    with response:
        status, size, etag = _response_metadata(response)
        if (status, size, etag) != (200, info["declared_bytes"], info["declared_etag"]):
            raise ValueError("GET metadata differs from approved HEAD")
        try:
            output = target.open("xb")
        except OSError as error:
            raise AcquisitionError("open_storage_archive", error) from error
        with output:
            while True:
                if time.monotonic() > deadline:
                    raise TimeoutError("YCB acquisition exceeded 900-second object deadline")
                try:
                    chunk = response.read(min(1 << 20, info["declared_bytes"] - count + 1))
                except OSError as error:
                    raise AcquisitionError("read_https_body", error) from error
                if not chunk:
                    break
                count += len(chunk)
                if count > info["declared_bytes"]:
                    raise ValueError("download exceeded declared compressed size")
                _reserve(stage, MAX_EXTRACT)
                try:
                    output.write(chunk)
                except OSError as error:
                    raise AcquisitionError("write_storage_archive", error) from error
                digest.update(chunk)
    observed = digest.hexdigest()
    if count != info["declared_bytes"] or (expected_sha256 is not None and observed != expected_sha256):
        raise ValueError("download size or independently supplied SHA-256 mismatch")
    return target, {"path": "archives/" + info["filename"], "bytes": count,
                    "observed_sha256": observed, "independently_pinned": expected_sha256 is not None,
                    "source": info}


def _photo_selection(object_id):
    return {f"{object_id}/NP3_{angle}.jpg": Path("photos") / f"NP3_{angle:03}.jpg"
            for angle in range(0, 360, 6)}


def _validate_photo(path):
    from PIL import Image
    if Path(path).stat().st_size > 32 * 1024**2:
        raise ValueError("selected photo exceeds JPEG size cap")
    with Image.open(path) as image:
        if image.format != "JPEG" or image.mode != "RGB" or image.size != PHOTO_SIZE:
            raise ValueError("selected NP3 JPEG has unexpected format, channels or dimensions")
        image.load()  # Detect a truncated or undecodable JPEG before publication.


def fetch(object_id, storage_root=STORAGE_ROOT, expected_berkeley_sha256=None,
          expected_google_sha256=None):
    """One-object transaction; final object directory appears only after validation."""
    if object_id not in OBJECTS:
        raise ValueError("unsupported YCB object")
    storage_root = Path(storage_root)
    if any(part.is_symlink() for part in (storage_root, *storage_root.parents)):
        raise ValueError("symlink storage destination refused")
    destination = storage_root / object_id
    if destination.exists() or destination.is_symlink():
        raise FileExistsError(destination)
    for value in (expected_berkeley_sha256, expected_google_sha256):
        if value is not None and (len(value) != 64 or any(ch not in "0123456789abcdef" for ch in value)):
            raise ValueError("expected SHA-256 must be 64 lowercase hex digits")
    head = preflight(object_id)
    storage_root.mkdir(parents=True, exist_ok=True)
    compressed_total = sum(info["declared_bytes"] for info in head["sources"].values())
    _reserve(storage_root, compressed_total + MAX_EXTRACT)
    deadline = time.monotonic() + MAX_SECONDS
    stage = Path(tempfile.mkdtemp(prefix=f".ycb-{object_id}-", dir=storage_root))
    try:
        paths, archives = {}, {}
        for role, expected in (("berkeley_rgbd", expected_berkeley_sha256),
                               ("google", expected_google_sha256)):
            paths[role], archives[role] = _download(stage, head["sources"][role], expected, deadline)
        photos = base.extract_selected(paths["berkeley_rgbd"], _photo_selection(object_id),
                                       stage, MAX_MEMBERS, MAX_EXTRACT, deadline)
        if len(photos) != 60:
            raise ValueError("selected NP3 photo count differs from 60")
        for record in photos:
            photo = stage / record["path"]
            with photo.open("rb") as stream:
                if stream.read(3) != b"\xff\xd8\xff":
                    raise ValueError("selected NP3 member lacks JPEG start marker")
                stream.seek(-2, 2)
                if stream.read() != b"\xff\xd9":
                    raise ValueError("selected NP3 member lacks JPEG end marker")
            _validate_photo(photo)
            record["role"] = "reconstruction-input-real-photograph"
            record["turntable_angle_degrees"] = int(Path(record["path"]).stem.split("_")[1])
        resolution = OBJECTS[object_id]["scan_resolution"]
        member = f"{object_id}/google_{resolution}/nontextured.ply"
        original_relative = Path("reference/google_original_ascii.ply")
        selected_scan = base.extract_selected(paths["google"], {member: original_relative},
                                              stage, 32, MAX_EXTRACT - sum(x["bytes"] for x in photos),
                                              deadline)[0]
        converted_path = stage / "reference/google_geometry_f64.ply"
        converted = base.convert_ascii_scan(stage / original_relative, converted_path, deadline)
        expanded = sum(x["bytes"] for x in photos) + selected_scan["bytes"] + converted["bytes"]
        if expanded > MAX_EXTRACT:
            raise ValueError("expanded selected subset exceeds 200 MB cap")
        if time.monotonic() > deadline:
            raise TimeoutError("YCB acquisition exceeded object deadline during scan conversion")
        _reserve(stage)
        for record in photos + [selected_scan]:
            if sha256_file(stage / record["path"]) != record["sha256"]:
                raise ValueError("selected member changed before publication")
        if sha256_file(converted_path) != converted["sha256"]:
            raise ValueError("converted scan changed before publication")
        if any(sha256_file(stage / value["path"]) != value["observed_sha256"]
               for value in archives.values()):
            raise ValueError("archive changed during extraction")
        manifest = {"schema": "ycb_more_prepared_v1", "object_id": object_id,
                    "status": "complete", "source_authentication": (
                        "independently_sha256_pinned" if all(x["independently_pinned"] for x in archives.values())
                        else "observed_sha256_only_unverified_upstream_identity"),
                    "producer_script_sha256": sha256_file(Path(__file__)),
                    "tar_converter_script_sha256": sha256_file(Path(base.__file__)),
                    "head_preflight": head, "archives": archives,
                    "selection": {"camera": "Berkeley NP3", "angles": list(range(0, 360, 6)),
                                  "views": 60, "excluded_from_reconstruction":
                                  ["depth", "masks", "provided poses", "Google scanner reference"]},
                    "photos": sorted(photos, key=lambda x: x["turntable_angle_degrees"]),
                    "reference_original": {**selected_scan, "role": "Google scanner reference only"},
                    "reference_binary_geometry": {**converted,
                        "path": "reference/" + converted["path"],
                        "role": "converted Google scanner reference only"},
                    "evaluation_caveat": "Separate-sensor shape oracle, not certified physical ground truth; no registration to NP3 frame here"}
        (stage / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        _reserve(stage)
        if destination.exists() or destination.is_symlink():
            raise FileExistsError(destination)
        stage.replace(destination)  # Same storage filesystem: no partial final directory.
        return manifest
    except BaseException as error:
        failure = {"schema": "ycb_more_fetch_failure_v1", "object_id": object_id,
                   "status": "failed_no_final_object", "phase": getattr(error, "phase", "other"),
                   "producer_script_sha256": sha256_file(Path(__file__)),
                   "tar_converter_script_sha256": sha256_file(Path(base.__file__)),
                   "error_type": type(error).__name__, "error_message": str(error)[:512],
                   "cause_type": type(error.__cause__).__name__ if error.__cause__ else None,
                   "cause_errno": getattr(error.__cause__, "errno", None),
                   "stage_archive_bytes": {p.name: p.stat().st_size
                                           for p in (stage / "archives").glob("*.tgz")},
                   "cleanup_pending": True}
        failure_path = storage_root / (stage.name + ".failure.json")
        try:
            encoded = (json.dumps(failure, indent=2) + "\n").encode()
            if len(encoded) <= 4096:
                with failure_path.open("xb") as stream:
                    stream.write(encoded)
        except OSError:
            pass  # Preserve the original failure even if diagnostic storage fails.
        try:
            shutil.rmtree(stage)
        except OSError:
            raise RuntimeError(f"fetch failed and stage cleanup also failed; inspect {stage}") from error
        raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--preflight", action="store_true", help="HEAD metadata only; no local output")
    mode.add_argument("--fetch", action="store_true", help="bounded GET and selected-member preparation")
    parser.add_argument("--object", choices=(*OBJECTS, "all"), required=True)
    parser.add_argument("--storage-root", type=Path, default=STORAGE_ROOT)
    parser.add_argument("--berkeley-sha256")
    parser.add_argument("--google-sha256")
    args = parser.parse_args()
    if args.preflight:
        selected = OBJECTS if args.object == "all" else (args.object,)
        print(json.dumps([preflight(object_id) for object_id in selected], indent=2))
    else:
        if args.object == "all":
            parser.error("fetch one object at a time for a separate transaction and deadline")
        report = fetch(args.object, args.storage_root, args.berkeley_sha256, args.google_sha256)
        print(json.dumps({"object_id": args.object, "photos": len(report["photos"]),
                          "destination": str(args.storage_root / args.object),
                          "source_authentication": report["source_authentication"]}))


if __name__ == "__main__":
    main()
