"""Read-only TRAIN OpenMVG geometric-match support audit against coarse masks.

Masks are accepted pose support, not object silhouettes. No filtering or
reconstruction is performed; stdout is the sole output.
"""
from __future__ import annotations

import hashlib
import json
import math
from pathlib import Path
import struct

from PIL import Image


BASE = Path("/Volumes/backups/code/crispmesh-data")
SFM = BASE / "openmvg-mustard-photo-sfm-001"
MASKS = BASE / "mustard-sfm-train-001"
PROFILE = Path(__file__).resolve().parents[2] / "tests/datasets/ycb_np3_full_turn_profile.json"
SFM_RECEIPT_SHA = "e45c400dcf71a3ba86fa22f7af2138418d6dc511fddcd6d8470ef33599804744"
STAGE_REPORT_SHA = "bbcba624f6a51f82e0ebdebf6e7e4cda10804bc8df34cfd0d5b3242983a225d0"
PROFILE_SHA = "bb00d17e1940f121dc12f7ddb31d4adf80b7e9a935f18e5535cfc1a2cde1998d"
MATCH_SHA = "1e2eebfee9a5326431f4d9c415d88c957e8e30b99899302e33f0397daca2becc"
SCENE_SHA = "8a61622cc22f76a6bb7dbcbf5a4e00725afd6a57b1e20095dabafab931112033"
MAX_MATCH_BYTES = 1 << 20
MAX_FEAT_BYTES = 256 << 10
MAX_MATCHES = 100_000


def sha(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def decode_matches(blob: bytes, views: int = 48) -> list[tuple[int, int, list[tuple[int, int]]]]:
    """Decode pinned cereal PortableBinary map<Pair,uint32-index-pair vector>."""
    if len(blob) > MAX_MATCH_BYTES or len(blob) < 9 or blob[0] not in (0, 1):
        raise ValueError("invalid or oversized portable match archive")
    endian = "<" if blob[0] else ">"
    offset = 1

    def take(fmt: str) -> tuple:
        nonlocal offset
        size = struct.calcsize(fmt)
        if size > len(blob) - offset:
            raise ValueError("truncated portable match archive")
        value = struct.unpack_from(fmt, blob, offset)
        offset += size
        return value

    (pair_count,) = take(endian + "Q")
    if pair_count > views * (views - 1) // 2:
        raise ValueError("impossible pair count")
    records = []
    previous = (-1, -1)
    total = 0
    for _ in range(pair_count):
        left, right = take(endian + "II")
        if not (0 <= left < right < views) or (left, right) <= previous:
            raise ValueError("invalid, duplicated, or unsorted view pair")
        previous = (left, right)
        (count,) = take(endian + "Q")
        total += count
        if count == 0 or total > MAX_MATCHES or count > (len(blob) - offset) // 8:
            raise ValueError("invalid or truncated indexed matches")
        matches = [take(endian + "II") for _ in range(count)]
        records.append((left, right, matches))
    if offset != len(blob):
        raise ValueError("trailing bytes in portable match archive")
    return records


def read_features(path: Path) -> list[tuple[float, float]]:
    if path.is_symlink() or not path.is_file() or path.stat().st_size > MAX_FEAT_BYTES:
        raise ValueError(f"invalid feature file: {path}")
    result = []
    for line in path.read_text().splitlines():
        row = line.split()
        if len(row) != 4:
            raise ValueError("feature row must be x y scale orientation")
        x, y, scale, angle = map(float, row)
        if not all(map(math.isfinite, (x, y, scale, angle))) or scale <= 0:
            raise ValueError("invalid feature values")
        result.append((x, y))
    if len(result) > 10_000:
        raise ValueError("feature row count exceeded")
    return result


def nearest_pixel(x: float, y: float, width: int, height: int) -> int | None:
    ix, iy = math.floor(x + 0.5), math.floor(y + 0.5)
    return iy * width + ix if 0 <= ix < width and 0 <= iy < height else None


def match_class(left: bool | None, right: bool | None) -> str:
    if left is None or right is None:
        return "out_of_frame"
    return "both_in" if left and right else "one_in" if left or right else "neither_in"


def distance_bin(distance: int) -> str:
    if distance == 1:
        return "1"
    if distance <= 4:
        return "2-4"
    if distance <= 9:
        return "5-9"
    if distance <= 19:
        return "10-19"
    return "20-29" if distance < 30 else "30"


def audit() -> dict:
    receipt_file = SFM / "receipt.json"
    stage_file = MASKS / "stage-report.json"
    if any(path.is_symlink() or not path.is_file() for path in (receipt_file, stage_file, PROFILE)):
        raise ValueError("missing or linked sealed receipt/profile")
    if (sha(receipt_file) != SFM_RECEIPT_SHA or sha(stage_file) != STAGE_REPORT_SHA or
            sha(PROFILE) != PROFILE_SHA):
        raise ValueError("sealed receipt/profile changed")
    receipt = json.loads(receipt_file.read_text())
    stage = json.loads(stage_file.read_text())
    profile = json.loads(PROFILE.read_text())
    scene_path = SFM / "matches/sfm_data.json"
    match_path = SFM / "matches/matches.e.bin"
    if (sha(scene_path) != SCENE_SHA or sha(match_path) != MATCH_SHA or
            receipt["output_inventory"]["matches/sfm_data.json"]["sha256"] != SCENE_SHA or
            receipt["output_inventory"]["matches/matches.e.bin"]["sha256"] != MATCH_SHA):
        raise ValueError("sealed scene/matches changed")
    scene = json.loads(scene_path.read_text())
    views = {row["key"]: row["value"]["ptr_wrapper"]["data"]["filename"]
             for row in scene["views"]}
    names = stage["train_names"]
    slots = profile["slots"]
    if (len(views) != 48 or set(views) != set(range(48)) or
            set(views.values()) != set(names) or len(set(names)) != 48 or
            len(slots) != 60 or len(set(slots)) != 60 or not set(names) <= set(slots) or
            set(stage["cleaned_masks"]) != set(names) or
            stage["mask_role"] != "accepted coarse pose support, not object silhouette or ground truth"):
        raise ValueError("TRAIN/mask/profile scope differs")
    features, masks = {}, {}
    for view_id, name in sorted(views.items()):
        feat = SFM / "matches" / (Path(name).stem + ".feat")
        mask = MASKS / "masks" / (name + ".png")
        expected = receipt["output_inventory"]["matches/" + feat.name]["sha256"]
        if feat.is_symlink() or sha(feat) != expected or mask.is_symlink() or \
                sha(mask) != stage["cleaned_masks"][name]["sha256"]:
            raise ValueError(f"sealed feature/mask changed: {name}")
        features[view_id] = read_features(feat)
        with Image.open(mask) as image:
            if image.mode != "L" or image.size != (1280, 1024):
                raise ValueError("mask format differs")
            pixels = image.tobytes()
        kept = pixels.count(255)
        if (kept != stage["cleaned_masks"][name]["kept_pixels"] or
                pixels.count(0) + kept != len(pixels)):
            raise ValueError("mask is not sealed binary pose support")
        masks[view_id] = pixels
    records = decode_matches(match_path.read_bytes())
    slot_index = {name: i for i, name in enumerate(slots)}
    by_distance = {str(d): {"possible_pairs": 0, "verified_pairs": 0, "matches": 0,
                            "both_in": 0, "one_in": 0, "neither_in": 0,
                            "out_of_frame": 0} for d in range(1, 31)}
    for i in range(48):
        for k in range(i + 1, 48):
            delta = abs(slot_index[views[i]] - slot_index[views[k]])
            by_distance[str(min(delta, 60 - delta))]["possible_pairs"] += 1
    pair_rows = []
    for left, right, matches in records:
        delta = abs(slot_index[views[left]] - slot_index[views[right]])
        distance = min(delta, 60 - delta)
        row = {"views": [left, right], "names": [views[left], views[right]],
               "cyclic_slots": distance, "matches": len(matches),
               "both_in": 0, "one_in": 0, "neither_in": 0, "out_of_frame": 0}
        for i, j in matches:
            if i >= len(features[left]) or j >= len(features[right]):
                raise ValueError("feature index outside sealed feature rows")
            x1, y1 = features[left][i]
            x2, y2 = features[right][j]
            p1 = nearest_pixel(x1, y1, 1280, 1024)
            p2 = nearest_pixel(x2, y2, 1280, 1024)
            label = match_class(None if p1 is None else masks[left][p1] == 255,
                                None if p2 is None else masks[right][p2] == 255)
            row[label] += 1
        bucket = by_distance[str(distance)]
        bucket["verified_pairs"] += 1
        for key in ("matches", "both_in", "one_in", "neither_in", "out_of_frame"):
            bucket[key] += row[key]
        pair_rows.append(row)
    coarse_bins = {}
    for distance, row in by_distance.items():
        bucket = coarse_bins.setdefault(distance_bin(int(distance)), {key: 0 for key in row})
        for key, value in row.items():
            bucket[key] += value
    if sha(match_path) != MATCH_SHA or sha(scene_path) != SCENE_SHA or \
            sha(receipt_file) != SFM_RECEIPT_SHA or sha(stage_file) != STAGE_REPORT_SHA or \
            sha(PROFILE) != PROFILE_SHA:
        raise ValueError("source seal changed during read-only audit")
    for name in names:
        feat = SFM / "matches" / (Path(name).stem + ".feat")
        mask = MASKS / "masks" / (name + ".png")
        if (sha(feat) != receipt["output_inventory"]["matches/" + feat.name]["sha256"] or
                sha(mask) != stage["cleaned_masks"][name]["sha256"]):
            raise ValueError(f"feature/mask changed during read-only audit: {name}")
    return {"role": "coarse pose-support overlap only, not bottle/board/background truth",
            "source_sha256": {"sfm_receipt": SFM_RECEIPT_SHA, "mask_stage": STAGE_REPORT_SHA,
                              "profile": PROFILE_SHA, "matches_e": MATCH_SHA,
                              "sfm_data": SCENE_SHA},
            "total_possible_pairs": 1128, "verified_pairs": len(records),
            "matches": sum(len(row[2]) for row in records),
            "by_distance": by_distance, "cyclic_bins": coarse_bins, "pairs": pair_rows}


if __name__ == "__main__":
    print(json.dumps(audit(), sort_keys=True, indent=2))
