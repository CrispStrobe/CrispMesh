"""Prepare v0.3.0 Brush masks from sealed YCB-008 data; never rewrite RGB or cameras."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import shutil
import time

from PIL import Image

from .lineage import RUN_008, validate_008
from .preflight import MIN_FREE, sha256, validate_dataset

ROOT = Path(__file__).resolve().parents[2]
MOUNT = Path("/Volumes/backups")
OUTPUT = MOUNT / "code/crispmesh-data/brush-ycb-masked-input-001"
SOURCE = RUN_008 / "dense"
MASKS = RUN_008 / "masks"
MAX_BYTES = 256 * 1024**2
BUFFER = 256 * 1024**2
MAX_SECONDS = 240


def release_mask_name(image_name: str) -> str:
    """v0.3.0 finds a sibling mask with the same *file stem*."""
    path = Path(image_name)
    if path.name != image_name or path.suffix.lower() not in (".jpg", ".jpeg", ".png"):
        raise ValueError("Brush v0.3 mask bridge requires a flat image basename")
    return path.stem + ".png"


def unique_release_mask_names(image_names: list[str]) -> dict[str, str]:
    mapping = {}
    for name in image_names:
        target = release_mask_name(name)
        if target in mapping:
            raise ValueError("Brush v0.3 mask-stem collision")
        mapping[target] = name
    return mapping


def validate_mask(mask: Path, image: Path, expected_size: tuple[int, int]) -> dict:
    if mask.is_symlink() or image.is_symlink() or not mask.is_file() or not image.is_file():
        raise ValueError("missing/symlinked mask or image")
    with Image.open(image) as rgb, Image.open(mask) as alpha:
        rgb.load()
        alpha.load()
        if rgb.size != expected_size or alpha.size != expected_size:
            raise ValueError("RGB/mask dimensions differ from sealed undistorted camera")
        if rgb.mode != "RGB" or rgb.format != "JPEG" or alpha.mode != "L" or alpha.format != "PNG":
            raise ValueError("expected RGB JPEG and 8-bit grayscale mask")
        histogram = alpha.histogram()
        if sum(histogram[1:255]) != 0 or histogram[255] == 0 or histogram[0] == 0:
            raise ValueError("mask must have both 0-ignore and 255-keep, no intermediate alpha")
        return {"size": list(rgb.size), "keep_pixels": histogram[255],
                "ignored_pixels": histogram[0]}


def preflight(output: Path = OUTPUT) -> dict:
    if output != OUTPUT or output.exists() or output.is_symlink() or output.parent.is_symlink():
        raise ValueError("output must be exact fresh external target")
    if not MOUNT.is_mount() or MOUNT.stat().st_dev == ROOT.stat().st_dev or not output.resolve().is_relative_to(MOUNT.resolve()):
        raise ValueError("external volume is not separately mounted")
    data = validate_dataset(SOURCE, output)
    lineage = validate_008(SOURCE, data["image_sha256"], data["model_file_sha256"])
    report_path = MASKS / "report.json"
    mask_report = json.loads(report_path.read_text())
    rows = mask_report.get("images", [])
    if len(rows) != 60 or {row["name"] for row in rows} != set(data["image_sha256"]):
        raise ValueError("sealed mask report does not cover exactly registered images")
    unique_release_mask_names([row["name"] for row in rows])
    masks = {}
    for row in rows:
        name = row["name"]
        source_mask = MASKS / row["native_mask_name"]
        image = SOURCE / "images" / name
        if sha256(source_mask) != row["mask_sha256"] or sha256(image) != row["undistorted_image_sha256"]:
            raise ValueError(f"source image/mask changed: {name}")
        statistics = validate_mask(source_mask, image, tuple(row["undistorted_size"]))
        target_name = release_mask_name(name)
        if target_name in masks:
            raise ValueError("Brush v0.3 mask-stem collision")
        masks[target_name] = {"source": str(source_mask), "sha256": row["mask_sha256"],
                              "registered_image": name, **statistics}
    total = sum((SOURCE / "images" / name).stat().st_size for name in data["image_sha256"])
    total += sum((SOURCE / "sparse" / name).stat().st_size for name in data["model_file_sha256"])
    total += sum(Path(item["source"]).stat().st_size for item in masks.values())
    if total > MAX_BYTES or shutil.disk_usage(MOUNT).free < MIN_FREE + MAX_BYTES + BUFFER:
        raise ValueError("bridge exceeds 256 MiB or external disk reserve")
    return {"schema": "brush_v030_ycb_mask_bridge_v1", "status": "preflight",
            "source_dataset": str(SOURCE), "output_dataset": str(output / "dataset"),
            "source_model_sha256": data["model_file_sha256"],
            "source_image_sha256": data["image_sha256"],
            "source_mask_report_sha256": sha256(report_path), "masks": dict(sorted(masks.items())),
            "lineage": lineage, "estimated_copy_bytes": total,
            "mask_semantics": "0 ignore / 255 keep; grayscale first channel is Brush alpha loss mask",
            "rgb_reencoded": False, "camera_reencoded": False,
            "training_split": "all 60 registered images; no hidden eval split",
            "quality_claim": False}


def prepare() -> dict:
    plan = preflight()
    OUTPUT.mkdir()
    dataset = OUTPUT / "dataset"
    start = time.monotonic()
    copied_bytes = 0

    def copy_checked(src: Path, dst: Path, digest: str) -> None:
        nonlocal copied_bytes
        size = src.stat().st_size
        if time.monotonic() - start > MAX_SECONDS:
            raise ValueError("240-second bridge deadline")
        if copied_bytes + size > MAX_BYTES or shutil.disk_usage(MOUNT).free < MIN_FREE + size + BUFFER:
            raise ValueError("per-copy byte/free-space cap")
        shutil.copy2(src, dst)
        copied_bytes += dst.stat().st_size
        if time.monotonic() - start > MAX_SECONDS or copied_bytes > MAX_BYTES:
            raise ValueError("post-copy deadline/byte cap")
        if shutil.disk_usage(MOUNT).free < MIN_FREE or sha256(src) != digest or sha256(dst) != digest:
            raise ValueError(f"post-copy disk floor/checksum: {src.name}")

    try:
        (dataset / "images").mkdir(parents=True)
        (dataset / "sparse").mkdir()
        (dataset / "masks").mkdir()
        for name, digest in plan["source_image_sha256"].items():
            copy_checked(SOURCE / "images" / name, dataset / "images" / name, digest)
        for name, digest in plan["source_model_sha256"].items():
            copy_checked(SOURCE / "sparse" / name, dataset / "sparse" / name, digest)
        for name, item in plan["masks"].items():
            copy_checked(Path(item["source"]), dataset / "masks" / name, item["sha256"])
            validate_mask(dataset / "masks" / name,
                          dataset / "images" / item["registered_image"], tuple(item["size"]))
        actual_bytes = sum(path.stat().st_size for path in dataset.rglob("*") if path.is_file())
        if actual_bytes != copied_bytes or copied_bytes != plan["estimated_copy_bytes"]:
            raise ValueError("copied dataset byte count differs from preflight")
        expected_files = ({f"images/{name}" for name in plan["source_image_sha256"]} |
                          {f"sparse/{name}" for name in plan["source_model_sha256"]} |
                          {f"masks/{name}" for name in plan["masks"]})
        actual_files = {path.relative_to(dataset).as_posix() for path in dataset.rglob("*") if path.is_file()}
        if actual_files != expected_files or any(path.is_symlink() for path in dataset.rglob("*")):
            raise ValueError("prepared dataset file set changed")
        # Catch source mutations after its individual file was copied.
        if sha256(MASKS / "report.json") != plan["source_mask_report_sha256"]:
            raise ValueError("sealed source mask report changed during bridge")
        for name, digest in plan["source_image_sha256"].items():
            if sha256(SOURCE / "images" / name) != digest or sha256(dataset / "images" / name) != digest:
                raise ValueError(f"source RGB changed after copy: {name}")
        for name, digest in plan["source_model_sha256"].items():
            if sha256(SOURCE / "sparse" / name) != digest or sha256(dataset / "sparse" / name) != digest:
                raise ValueError(f"source model changed after copy: {name}")
        for name, item in plan["masks"].items():
            if (sha256(Path(item["source"])) != item["sha256"] or
                    sha256(dataset / "masks" / name) != item["sha256"]):
                raise ValueError("source mask changed after copy")
        if (time.monotonic() - start > MAX_SECONDS or
                shutil.disk_usage(MOUNT).free < MIN_FREE):
            raise ValueError("bridge deadline or external disk floor crossed")
        if shutil.disk_usage(MOUNT).free < MIN_FREE:
            raise ValueError("external 10 GiB free floor crossed")
        plan["status"] = "complete"
        plan["copied_bytes"] = copied_bytes
    except Exception as exc:
        plan["status"] = "failed"
        plan["failure"] = f"{type(exc).__name__}: {exc}"
        plan["copied_bytes"] = copied_bytes
        with (OUTPUT / "bridge-report.json").open("x") as stream:
            json.dump(plan, stream, indent=2, sort_keys=True)
            stream.write("\n")
        raise
    with (OUTPUT / "bridge-report.json").open("x") as stream:
        json.dump(plan, stream, indent=2, sort_keys=True)
        stream.write("\n")
    return plan


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prepare", action="store_true", help="write fresh external dataset; only after review")
    args = parser.parse_args()
    report = prepare() if args.prepare else preflight()
    print(json.dumps({key: report[key] for key in ("status", "output_dataset", "estimated_copy_bytes",
                                                   "source_mask_report_sha256")}, indent=2))


if __name__ == "__main__":
    main()
