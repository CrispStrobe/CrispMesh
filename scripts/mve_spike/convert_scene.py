#!/usr/bin/env python3
"""Test-only conversion of measured CrispMesh sparse geometry into an MVE scene."""

import argparse
import hashlib
import json
import math
from pathlib import Path
import shutil


RESERVE = 10 * 1024**3


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def camera_coordinates(rotation, translation, point):
    return [sum(rotation[3 * row + col] * point[col] for col in range(3)) + translation[row]
            for row in range(3)]


def project(calibration, rotation, translation, point):
    x, y, z = camera_coordinates(rotation, translation, point)
    if z <= 0:
        raise ValueError("track is behind camera")
    return (calibration["fx"] * x / z + calibration["cx"],
            calibration["fy"] * y / z + calibration["cy"])


def mve_intrinsics(calibration):
    width, height = calibration["width"], calibration["height"]
    fx, fy = calibration["fx"], calibration["fy"]
    if width <= height or fx <= 0 or fy <= 0 or (width / height) * (fy / fx) < 1:
        raise ValueError("fixture requires positive focal lengths and landscape image")
    # MVE K has fx=flen*width, fy=flen*width*paspect, and subtracts
    # 0.5 after projection. Compensate in normalized principal point.
    flen = fx / width
    paspect = fy / fx
    return flen, paspect, (calibration["cx"] + 0.5) / width, (calibration["cy"] + 0.5) / height


def convert(fixture, scene):
    fixture = fixture.resolve(strict=True)
    if scene.exists() or scene.is_symlink():
        raise FileExistsError(scene)
    scene.parent.mkdir(parents=True, exist_ok=True)
    project_data = json.loads((fixture / "project.json").read_text())
    sparse = json.loads((fixture / "sparse-report.json").read_text())
    calibration = project_data["calibration"]
    if (sparse["coordinateFrame"] != "board" or sparse["units"] != "mm" or
            calibration["distortion"] != [0, 0, 0, 0]):
        raise ValueError("expected undistorted board-frame metric fixture")
    flen, paspect, ppx, ppy = mve_intrinsics(calibration)
    views = sparse["views"]
    if len(views) < 2 or len(views) > 20 or len(views) != len(project_data["images"]):
        raise ValueError("view count mismatch")
    if len(sparse["points"]) > 100_000:
        raise ValueError("too many sparse seeds for bounded spike")
    image_by_id = {image["id"]: image for image in project_data["images"]}
    for image in image_by_id.values():
        image_name = image["path"]
        if Path(image_name).name != image_name or not image_name.endswith(".png"):
            raise ValueError("fixture image path must be a local PNG basename")
    view_ids = {view["imageId"]: index for index, view in enumerate(views)}
    if len(view_ids) != len(views):
        raise ValueError("duplicate view")
    checked = []
    for point in sparse["points"]:
        if len(point["observations"]) < 2:
            raise ValueError("sparse seed requires at least two observations")
        for observation in point["observations"]:
            view = views[view_ids[observation["imageId"]]]
            u, v = project(calibration, view["rotation"], view["translationMm"], point["positionMm"])
            error = math.hypot(u - observation["pixel"][0], v - observation["pixel"][1])
            checked.append(error)
    if not checked or max(checked) > 2:
        raise ValueError(f"sparse reprojection check failed: max={max(checked) if checked else None}")
    image_bytes = sum((fixture / image_by_id[view["imageId"]]["path"]).stat().st_size for view in views)
    if shutil.disk_usage(scene.parent).free < RESERVE + image_bytes + 32 * 1024 * 1024:
        raise RuntimeError("10 GiB free-space reserve")
    scene.mkdir()
    views_dir = scene / "views"
    views_dir.mkdir()
    source_hashes = {"project.json": digest(fixture / "project.json"),
                     "sparse-report.json": digest(fixture / "sparse-report.json")}
    bundle_lines = ["drews 1.0", f"{len(views)} {len(sparse['points'])}"]
    for index, view in enumerate(views):
        image = image_by_id[view["imageId"]]
        image_path = fixture / image["path"]
        source_hashes[image["path"]] = digest(image_path)
        directory = views_dir / f"view_{index:04d}.mve"
        directory.mkdir()
        shutil.copyfile(image_path, directory / "undistorted.png")
        rotation = view["rotation"]
        translation = view["translationMm"]
        lines = ["[view]", f"id = {index}", f"name = {view['imageId']}",
                 "[camera]", f"focal_length = {flen:.12g}", "radial_distortion = 0 0",
                 f"pixel_aspect = {paspect:.12g}", f"principal_point = {ppx:.12g} {ppy:.12g}",
                 "rotation = " + " ".join(f"{value:.12g}" for value in rotation),
                 "translation = " + " ".join(f"{value:.12g}" for value in translation)]
        (directory / "meta.ini").write_text("\n".join(lines) + "\n")
        bundle_lines.append(f"{flen:.12g} 0 0")
        for row in range(3):
            bundle_lines.append(" ".join(f"{rotation[3 * row + col]:.12g}" for col in range(3)))
        bundle_lines.append(" ".join(f"{value:.12g}" for value in translation))
    for point in sparse["points"]:
        bundle_lines.append(" ".join(f"{value:.12g}" for value in point["positionMm"]))
        bundle_lines.append("255 255 255")
        refs = [f"{view_ids[obs['imageId']]} {point['id']} 0" for obs in point["observations"]]
        bundle_lines.append(f"{len(refs)} " + " ".join(refs))
    (scene / "synth_0.out").write_text("\n".join(bundle_lines) + "\n")
    summary = {"source_hashes_sha256": source_hashes, "views": len(views),
               "sparse_seeds": len(sparse["points"]), "observations": len(checked),
               "reprojection_rms_px": math.sqrt(sum(error * error for error in checked) / len(checked)),
               "reprojection_max_px": max(checked),
               "mve_intrinsics": {"focal_length": flen, "pixel_aspect": paspect,
                                  "principal_point": [ppx, ppy]},
               "mask_enforcement": "unavailable in selected MVE dense path"}
    (scene / "conversion.json").write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n")
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--scene", type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(convert(args.fixture, args.scene), indent=2))


if __name__ == "__main__":
    main()
