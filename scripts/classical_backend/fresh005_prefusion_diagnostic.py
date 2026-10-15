"""Read-only, reference-free inventory of the sealed fresh 005 OpenMVS DMAPs.

This describes input pixels, not which pixels survived multi-view fusion. It
does not launch OpenMVS, select a filter, or infer physical box dimensions.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import struct

import numpy as np

from scripts.classical_backend.run import digest


SOURCE = Path("/Volumes/backups/code/crispmesh-data/turntable-fresh-openmvs-005")
RESULT_SHA256 = "dafc6ccb2a90f0c5a45748291571455237acd8ae08da01dd80adc10222a3625e"
HEADER = struct.Struct("<HBBIIIIff")
QUANTILES = (0.01, 0.5, 0.99)


def _real_file(path: Path) -> None:
    if path.is_symlink() or not path.is_file() or path.stat().st_size == 0:
        raise ValueError(f"missing, empty, or linked file: {path}")


def inspect_map(path: Path, expected_name: str, expected_sha256: str) -> dict:
    """Map one validated payload without loading the full collection into RAM."""
    _real_file(path)
    if digest(path) != expected_sha256:
        raise ValueError(f"DMAP differs from sealed camera check: {path.name}")
    with path.open("rb") as stream:
        raw = stream.read(HEADER.size)
        if len(raw) != HEADER.size:
            raise ValueError("truncated DMAP header")
        magic, flags, _, image_w, image_h, width, height, low, high = HEADER.unpack(raw)
        if (magic != 0x5244 or flags != 7 or not 0 < width <= image_w <= 8192 or
                not 0 < height <= image_h <= 8192 or not np.isfinite([low, high]).all()):
            raise ValueError("unexpected DMAP header, dimensions, or channels")
        name_size_raw = stream.read(2)
        if len(name_size_raw) != 2:
            raise ValueError("truncated DMAP image name")
        name_size = struct.unpack("<H", name_size_raw)[0]
        if not 0 < name_size < 4096:
            raise ValueError("invalid DMAP image name size")
        name = stream.read(name_size).decode("utf-8")
        if name != "dense/images/" + expected_name:
            raise ValueError("DMAP image name differs from camera check")
        raw_count = stream.read(4)
        if len(raw_count) != 4:
            raise ValueError("truncated DMAP view count")
        view_count = struct.unpack("<I", raw_count)[0]
        if not 1 <= view_count <= 200:
            raise ValueError("invalid DMAP view count")
        if len(stream.read(view_count * 4 + 21 * 8)) != view_count * 4 + 21 * 8:
            raise ValueError("truncated DMAP camera")
        payload_start = stream.tell()
    pixels = width * height
    if path.stat().st_size != payload_start + pixels * (4 + 12 + 4):
        raise ValueError("DMAP payload length differs from depth/normal/confidence layout")
    depth = np.memmap(path, dtype="<f4", mode="r", offset=payload_start, shape=(pixels,))
    confidence = np.memmap(path, dtype="<f4", mode="r", offset=payload_start + pixels * 16,
                           shape=(pixels,))
    if (not np.isfinite(depth).all() or np.any(depth < 0) or
            not np.isfinite(confidence).all() or np.any(confidence < 0)):
        raise ValueError("nonfinite or negative depth/confidence")
    positive = depth > 0
    values = depth[positive]
    conf_values = confidence[positive]
    if not len(values):
        raise ValueError("DMAP has no positive depth")
    # Quantiles describe the native camera-depth and confidence scales. They
    # are observational summaries, never tuned thresholds or quality scores.
    return {"name": expected_name, "sha256": expected_sha256,
            "image_size": [image_w, image_h], "depth_size": [width, height],
            "neighbor_view_ids": view_count - 1, "pixels": pixels,
            "positive_depth_pixels": int(np.count_nonzero(positive)),
            "positive_depth_fraction": float(np.mean(positive)),
            "positive_depth_zero_confidence": int(np.count_nonzero(conf_values == 0)),
            "positive_depth_quantiles": np.quantile(values, QUANTILES).tolist(),
            "positive_depth_confidence_quantiles": np.quantile(conf_values, QUANTILES).tolist()}


def diagnose(source: Path = SOURCE, expected_result_sha256: str = RESULT_SHA256) -> dict:
    if source.is_symlink() or not source.is_dir():
        raise ValueError("missing or linked source run")
    result_path = source / "result.json"
    check_path = source / "dmap-camera-check.json"
    mask_path = source / "masks/report.json"
    for path in (result_path, check_path, mask_path, source / "dense.ply"):
        _real_file(path)
    if digest(result_path) != expected_result_sha256:
        raise ValueError("fresh 005 result seal differs")
    result = json.loads(result_path.read_text())
    check_sha = result.get("artifact_sha256", {}).get("dmap-camera-check.json")
    mask_stages = [row for row in result.get("stages", []) if row.get("name") == "warp_masks"]
    if (result.get("schema") != "classical_fresh_masked_complete_v1" or
            result.get("status") != "complete" or result.get("sources_unchanged") is not True or
            len(mask_stages) != 1 or mask_stages[0].get("status") != "complete" or
            digest(check_path) != check_sha or
            digest(mask_path) != mask_stages[0].get("artifact", {}).get("report_sha256") or
            digest(source / "dense.ply") != result["artifact_sha256"].get("dense.ply")):
        raise ValueError("fresh run, mask, camera check, or dense control is unsealed")
    check = json.loads(check_path.read_text())
    masks = json.loads(mask_path.read_text())
    rows = check.get("images", [])
    if (check.get("status") != "complete" or check.get("checked_views") != 60 or
            check.get("positive_depth_outside_mask") != 0 or len(rows) != 60 or
            masks.get("status") != "complete" or masks.get("ignore_mask_label") != 0 or
            len(masks.get("images", [])) != 60):
        raise ValueError("60-view camera/mask inclusion proof is incomplete")
    map_paths = sorted(source.glob("depth[0-9][0-9][0-9][0-9].dmap"))
    if [path.name for path in map_paths] != [f"depth{i:04d}.dmap" for i in range(1, 61)]:
        raise ValueError("final depth-map sequence is incomplete")
    if [row["name"] for row in rows] != sorted(row["name"] for row in rows) or \
            len({row["name"] for row in rows}) != 60 or \
            {row["name"] for row in rows} != {row["name"] for row in masks["images"]}:
        raise ValueError("camera/mask image sets or camera-check ordering differ")
    maps = [inspect_map(path, row["name"], row["dmap_sha256"])
            for path, row in zip(map_paths, rows)]
    if digest(result_path) != expected_result_sha256 or digest(check_path) != check_sha or \
            digest(mask_path) != mask_stages[0]["artifact"]["report_sha256"]:
        raise ValueError("source reports changed during reading")
    return {"schema": "fresh005_prefusion_diagnostic_v1", "status": "complete",
            "source": str(source.resolve()), "result_sha256": expected_result_sha256,
            "dmap_camera_check_sha256": check_sha,
            "mask_report_sha256": mask_stages[0]["artifact"]["report_sha256"],
            "dense_control_sha256": result["artifact_sha256"]["dense.ply"],
            "views": 60, "positive_depth_pixels": sum(row["positive_depth_pixels"] for row in maps),
            "pixels": sum(row["pixels"] for row in maps),
            "positive_depth_outside_mask": 0,
            "mapping_to_fused_points": "unavailable: PLY and DMAP reports do not retain per-point source pixel IDs",
            "interpretation": "camera-depth/confidence distributions only; no physical size or fusion attribution",
            "maps": maps}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, default=SOURCE)
    args = parser.parse_args()
    print(json.dumps(diagnose(args.source), indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
