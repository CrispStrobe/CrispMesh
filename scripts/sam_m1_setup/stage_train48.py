"""Bounded, hash-verified photo-only staging of frozen mustard TRAIN views.

No model, mask, held-out image, reference geometry, or inference is used.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import shlex
import shutil
import subprocess
import time

from scripts.classical_backend import run as bounded
from scripts.classical_backend.mustard_stage import (
    ACQUISITION_SHA256, HELDOUT_NAMES, PACKAGE, PACKAGE_SHA256, TRAIN_NAMES,
    sha256,
)

ROOT = Path(__file__).resolve().parents[2]
MOUNT = Path("/Volumes/backups")
OUTPUT = MOUNT / "code/crispmesh-data/sam21-m1-train48-001"
REMOTE_PHOTOS = "/mnt/storage/crispmesh-data/ycb-expansion-001/006_mustard_bottle/photos"
CAP = 150 * 1024**2
FLOOR = 10 * 1024**3
BUFFER = 256 * 1024**2
DEADLINE_SECONDS = 240


def remote_inventory() -> dict:
    """Read-only exact-name inventory from the approved VPS source directory."""
    script = (
        "import hashlib,json,pathlib;"
        f"base=pathlib.Path({REMOTE_PHOTOS!r});names={list(TRAIN_NAMES)!r};"
        "out={};"
        "exec('for name in names:\\n p=base/name\\n if p.is_symlink() or not p.is_file(): raise ValueError(name)\\n h=hashlib.sha256()\\n with p.open(\"rb\") as stream:\\n  for block in iter(lambda: stream.read(1048576), b\"\"): h.update(block)\\n out[name]={\"bytes\":p.stat().st_size,\"sha256\":h.hexdigest()}');"
        "print(json.dumps(out,sort_keys=True))"
    )
    command = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8", "vps",
               "python3 -c " + shlex.quote(script)]
    result = subprocess.run(command, capture_output=True, text=True, timeout=45, check=True)
    if len(result.stdout) > 16_384:
        raise ValueError("remote inventory response too large")
    return json.loads(result.stdout)


def expected_photos(package: dict) -> dict:
    training = package.get("training_inputs", [])
    heldout = package.get("heldout_photos", [])
    if (package.get("schema") != "ycb_object_evaluation_package_v1" or
            package.get("object_id") != "006_mustard_bottle" or
            package.get("manifest_sha256") != ACQUISITION_SHA256 or
            [row.get("path") for row in training] != [f"photos/{name}" for name in TRAIN_NAMES] or
            [row.get("path") for row in heldout] != [f"photos/{name}" for name in HELDOUT_NAMES]):
        raise ValueError("package is not the frozen exact 48-train/12-heldout mustard split")
    return {name: {"bytes": row["bytes"], "sha256": row["sha256"]}
            for name, row in zip(TRAIN_NAMES, training)}


def validate_remote_inventory(observed: dict, expected: dict) -> None:
    if observed != expected:
        raise ValueError("VPS TRAIN photo inventory differs from frozen package")


def preflight(output: Path = OUTPUT, mount: Path = MOUNT, root: Path = ROOT,
              package_path: Path = PACKAGE, inventory: dict | None = None) -> dict:
    if output.exists() or output.is_symlink() or not output.parent.is_dir() or output.parent.is_symlink():
        raise ValueError("stage output must be fresh under an existing real parent")
    if (not mount.is_mount() or mount.is_symlink() or
            mount.stat().st_dev == root.stat().st_dev or
            output.parent.stat().st_dev != mount.stat().st_dev or
            not output.resolve().is_relative_to(mount.resolve())):
        raise ValueError("stage must use mounted external volume distinct from workspace")
    if (shutil.disk_usage(mount).free < FLOOR + CAP + BUFFER or
            shutil.disk_usage(root).free < FLOOR):
        raise ValueError("150 MiB cap+buffer or 10 GiB dual-disk reserve unavailable")
    if package_path.is_symlink() or not package_path.is_file() or sha256(package_path) != PACKAGE_SHA256:
        raise ValueError("frozen mustard package SHA differs")
    expected = expected_photos(json.loads(package_path.read_text()))
    if sum(row["bytes"] for row in expected.values()) > CAP:
        raise ValueError("48 TRAIN photos exceed 150 MiB stage cap")
    observed = remote_inventory() if inventory is None else inventory
    validate_remote_inventory(observed, expected)
    names_bytes = ("\n".join(TRAIN_NAMES) + "\n").encode("ascii")
    return {"schema": "sam21_m1_train48_photos_v1", "status": "preflight",
            "package_sha256": PACKAGE_SHA256, "runner_sha256": sha256(Path(__file__)),
            "photo_sha256": {name: row["sha256"] for name, row in expected.items()},
            "photo_bytes": {name: row["bytes"] for name, row in expected.items()},
            "train_names": list(TRAIN_NAMES), "excluded_heldout_names": list(HELDOUT_NAMES),
            "train_names_sha256": hashlib.sha256(names_bytes).hexdigest(),
            "source": "vps:" + REMOTE_PHOTOS, "output": str(output),
            "max_bytes": CAP, "deadline_seconds": DEADLINE_SECONDS, "min_free_each_bytes": FLOOR,
            "contains_masks_or_reference_geometry": False}


def execute(output: Path = OUTPUT) -> dict:
    started = time.monotonic()
    report = preflight(output)
    output.mkdir()
    (output / "photos").mkdir()
    shutil.copy2(PACKAGE, output / "package.json")
    (output / "train-names.txt").write_bytes(("\n".join(TRAIN_NAMES) + "\n").encode("ascii"))
    report["status"] = "running"
    report["stages"] = []
    try:
        stage = bounded.stage(output, "rsync-train48",
                              ["rsync", "-a", "--files-from", str(output / "train-names.txt"),
                               "vps:" + REMOTE_PHOTOS + "/", str(output / "photos") + "/"],
                              started + DEADLINE_SECONDS, CAP, 1 << 20, 1 << 30,
                              extra_reserve_paths=(ROOT,))
        report["stages"].append(stage)
        photos = output / "photos"
        if ({p.name for p in photos.iterdir()} != set(TRAIN_NAMES) or
                any(p.is_symlink() or not p.is_file() for p in photos.iterdir())):
            raise ValueError("staged photos contain missing/extra/linked or held-out files")
        for name in TRAIN_NAMES:
            path = photos / name
            if (path.stat().st_size != report["photo_bytes"][name] or
                    sha256(path) != report["photo_sha256"][name]):
                raise ValueError(f"staged TRAIN photo differs: {name}")
        if (remote_inventory() != {name: {"bytes": report["photo_bytes"][name],
                                         "sha256": report["photo_sha256"][name]} for name in TRAIN_NAMES} or
                sha256(PACKAGE) != PACKAGE_SHA256 or sha256(output / "package.json") != PACKAGE_SHA256 or
                sha256(output / "train-names.txt") != report["train_names_sha256"] or
                sha256(Path(__file__)) != report["runner_sha256"] or
                bounded.folder_bytes(output) > CAP or time.monotonic() > started + DEADLINE_SECONDS or
                min(shutil.disk_usage(MOUNT).free, shutil.disk_usage(ROOT).free) < FLOOR):
            raise ValueError("stage postflight source, output, runner, cap, or dual-disk gate failed")
        report["status"] = "complete"
        report["output_bytes"] = bounded.folder_bytes(output)
    except BaseException as error:
        if isinstance(error, bounded.StageError):
            report["stages"].append(error.result)
        report["status"] = "failed"
        report["failure"] = f"{type(error).__name__}: {error}"
        (output / "stage-report.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
        raise
    (output / "stage-report.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    return report


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--execute", action="store_true", help="copy only after explicit reviewed approval")
    args = parser.parse_args()
    result = execute() if args.execute else preflight()
    print(json.dumps({key: result[key] for key in ("status", "output", "package_sha256")}, indent=2))


if __name__ == "__main__":
    main()
