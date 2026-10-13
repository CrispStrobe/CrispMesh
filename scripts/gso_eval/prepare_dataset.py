"""Builds the folder of a public example-objects dataset from rendered GSO captures.

    python prepare_dataset.py OUT NAME:TITLE:MODEL:CAPTURE_DIR [...]

CAPTURE_DIR holds photos/shot_NNN.png and lens.json from `crisp3ds-dense render`. Writes
OUT/<name>/rgb/<name>_<n>_rgb.png, OUT/calibration/render-3dlf-lens.json, OUT/manifest.json
(schema crisp3ds_example_objects_v1, as cstr/3dlf-scan-photos) and OUT/README.md (dataset card).
"""
import hashlib
import json
import shutil
import sys
from pathlib import Path

ATTRIBUTION = (
    "Rendered from Google Scanned Objects by Google LLC (Copyright 2020 Google LLC), licensed CC BY 4.0 "
    "(https://creativecommons.org/licenses/by/4.0/); Downs et al., \"Google Scanned Objects: A High-Quality "
    "Dataset of 3D Scanned Household Items\", ICRA 2022. Changes: the scanned meshes were rendered as turntable "
    "photo sequences by Crisp3DS (crisp3ds-dense render); the photos are synthetic, not photographs."
)


def digest(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def entry(root, path):
    return {"path": path.relative_to(root).as_posix(), "size": path.stat().st_size, "sha256": digest(path)}


def main():
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=False)
    (out / "calibration").mkdir()
    objects, rows, lens_text = [], [], None
    for spec in sys.argv[2:]:
        name, title, model, capture = spec.split(":", 3)
        capture = Path(capture)
        text = (capture / "lens.json").read_text()
        if lens_text is None:
            lens_text = text
        elif json.loads(text) != json.loads(lens_text):
            raise SystemExit(f"{name}: lens differs from the first object's")
        photos = sorted((capture / "photos").glob("shot_*.png"))
        folder = out / name / "rgb"
        folder.mkdir(parents=True)
        files = []
        for n, photo in enumerate(photos):
            target = folder / f"{name}_{n}_rgb.png"
            shutil.copyfile(photo, target)
            files.append(entry(out, target))
        objects.append({"name": name, "title": title, "photos": len(files),
                        "bytes": sum(f["size"] for f in files), "calibration": "calibration/render-3dlf-lens.json",
                        "source_model": f"https://fuel.gazebosim.org/1.0/GoogleResearch/models/{model}",
                        "files": files})
        rows.append((name, title, model, len(files), sum(f["size"] for f in files)))
    lens = json.loads(lens_text)
    lens["provenance"] = {
        "source": "crisp3ds-dense render: the lens the photos were rendered with",
        "note": "The 3DLF apiCAM PRO lens (cstr/3dlf-scan-photos calibration/3dlf-pro.json) scaled to 1749 x 1155; "
                "the photos were rendered through exactly this model, so it is exact for them.",
    }
    calibration = out / "calibration/render-3dlf-lens.json"
    calibration.write_text(json.dumps(lens, indent=2) + "\n")
    for o in objects:
        o["bytes"] += calibration.stat().st_size
    manifest = {
        "schema": "crisp3ds_example_objects_v1",
        "source": "https://goo.gle/scanned-objects",
        "license": "CC-BY-4.0",
        "attribution": ATTRIBUTION,
        "calibration_files": [entry(out, calibration)],
        "objects": objects,
    }
    (out / "manifest.json").write_text(json.dumps(manifest, indent=1) + "\n")
    table = "\n".join(f"| {n} | {t} | [{m}](https://fuel.gazebosim.org/1.0/GoogleResearch/models/{m}) | {c} | "
                      f"{b / 1e6:.0f} MB |" for n, t, m, c, b in rows)
    card = f"""---
license: cc-by-4.0
pretty_name: Rendered turntable photos of Google Scanned Objects
task_categories:
  - image-to-3d
tags:
  - photogrammetry
  - turntable
  - multi-view
  - 3d-reconstruction
  - synthetic
size_categories:
  - n<1K
---

# Rendered turntable photos of Google Scanned Objects

Turntable photo sequences **rendered** from a few objects of
[Google Scanned Objects](https://goo.gle/scanned-objects), for trying photo-to-3D reconstruction
without a camera. The photos are synthetic: each scanned, textured mesh stands on a light disc in
front of a light backdrop and is seen from one ring of 72 cameras 20 degrees above it (5 degree
steps), through a real lens model (the 3DLF apiCAM PRO lens with its radial distortion), at
1749 x 1155 pixels, with one light that keeps its place relative to the camera, a cast shadow,
1 px Gaussian blur and mild noise. Rendered by `crisp3ds-dense render` of
[CrispMesh](https://github.com/CrispStrobe/crisp3ds), which uses them as downloadable
example objects.

| Object | Title | Source model | Photos | Size |
| --- | --- | --- | --- | --- |
{table}

Layout: `<object>/rgb/<object>_<n>_rgb.png` in capture order and
`calibration/render-3dlf-lens.json` (the lens the photos were rendered with, in Crisp3DS's
`crisp3ds_lens_calibration_v1` format; exact for these photos). `manifest.json` lists every file
with its size and SHA-256.

## Source and license

The objects are from **Google Scanned Objects**, Copyright 2020 Google LLC, licensed
[CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) (as stated in each model's
`model.config` and `metadata.pbtxt`). Dataset paper: Laura Downs, Anthony Francis, Nate Koenig,
Brandon Kinman, Ryan Hickman, Krista Reymann, Thomas B. McHugh, Vincent Vanhoucke,
"Google Scanned Objects: A High-Quality Dataset of 3D Scanned Household Items", ICRA 2022,
<https://arxiv.org/abs/2204.11918>. Models downloaded from Gazebo Fuel (owner GoogleResearch).

These photos are licensed CC BY 4.0 as well. Changes: the meshes and textures were rendered into
photographs; no mesh or texture is redistributed here. Product names and brands visible on the
objects belong to their owners; their appearance here implies no endorsement.
"""
    (out / "README.md").write_text(card)
    print(json.dumps([{"name": o["name"], "photos": o["photos"], "bytes": o["bytes"]} for o in objects]))


if __name__ == "__main__":
    main()
