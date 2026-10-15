"""Fixed, bounded external-volume SAM 2.1 M1 setup. Never runs inference.

The selected model code and checkpoint are copied from the already reviewed VPS
artifacts. The only pip inputs are hash-pinned macOS wheels; two pure-Python
packages without upstream wheels are copied from the VPS as an explicit vendor
exception. Nothing is installed into the system Python.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import signal
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
MOUNT = Path("/Volumes/backups")
AI = MOUNT / "ai/crispmesh-sam21-m1-002"
CODE = MOUNT / "code/crispmesh-data"
SOURCE = CODE / "sam21-m1-source-002"
DATA = CODE / "sam21-m1-three-train-002"
LOCK = Path(__file__).with_name("wheels.lock")
LOCK_SHA = "3c07dc415436bf22c09d597ca39aead5709f935e04729a9ad6aa25deed9b9a3d"
PACKAGE = ROOT / "build-opencv/ycb-evaluation-vps-001/mustard-package.json"
PROMPTS = ROOT / "tests/datasets/sam21_mustard_point48_prompts.json"
REMOTE = "/mnt/storage/crispmesh-data"
PREREQ = REMOTE + "/sam21-tiny-prereq-001"
PRIOR = REMOTE + "/sam21-mustard-point-full-001"
PHOTOS = REMOTE + "/ycb-expansion-001/006_mustard_bottle/photos"
NAMES = ("NP3_000.jpg", "NP3_066.jpg", "NP3_108.jpg")
SOURCE_SHA = "c7eb4585a22dadd4f54ffd9134e3103c1951745a3b4631ceb0684b55742069e3"
MODEL_SHA = "7402e0d864fa82708a20fbd15bc84245c2f26dff0eb43a4b5b93452deb34be69"
VENDOR_SHA = "dc8f6a3d428740eec66326890290673f77eea52a1fc040bb8c3458bed108da09"
PACKAGE_SHA = "45b6ed1430627a747eaeb8d4dff7d2c49502083cca0a7914e25aacd45b594425"
PROMPTS_SHA = "28440795d5f38590df8c5568969970d6623fbe22ea6b3e8781cba10608149d32"
PRIOR_SHA = "0ff3c6203301988890ded40276c58d510582bb08a7fa2f1884d2ba792349fd09"
BATCH_SHA = ("ec40a87c334f1168da4e56c54dd7373dd8650bd4b71a4bdcf70a124b580436c8",
             "2f0c8d51ee40e14c3dd57679d868fae01167563479abde29fbfc262bce839ee0")
CAP = 2 * 1024**3
RESERVE = 10 * 1024**3
BUFFER = 256 * 1024**2
SECONDS = 600
LOG_CAP = 2 * 1024**2
VENDOR_DIRS = ("iopath", "antlr4", "iopath-0.1.10.dist-info",
               "antlr4_python3_runtime-4.9.3.dist-info")


def sha(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def bytes_under(path: Path) -> int:
    if not path.exists():
        return 0
    total = 0
    for base, dirs, files in os.walk(path, followlinks=False):
        for name in dirs + files:
            member = Path(base) / name
            if member.is_symlink():
                raise ValueError(f"setup contains symlink: {member}")
            if member.is_file():
                total += member.stat().st_size
            elif not member.is_dir():
                raise ValueError(f"setup contains special file: {member}")
    return total


def source_digest(source: Path) -> str:
    files = sorted((source / "sam2").rglob("*.py")) + sorted((source / "sam2/configs").rglob("*.yaml"))
    files += [source / "LICENSE"]
    if not 30 <= len(files) <= 500:
        raise ValueError("wrong selected source count")
    h = hashlib.sha256()
    for file in sorted(files):
        if file.is_symlink() or not file.is_file() or file.stat().st_size > 2 * 1024**2:
            raise ValueError("source file missing, linked, or oversize")
        h.update(file.relative_to(source).as_posix().encode() + b"\0" + bytes.fromhex(sha(file)))
    return h.hexdigest()


def vendor_inventory(vendor: Path) -> dict:
    if {p.name for p in vendor.iterdir()} != set(VENDOR_DIRS):
        raise ValueError("vendor must contain exactly the two reviewed pure-Python packages and metadata")
    files = {}
    for base, dirs, names in os.walk(vendor, followlinks=False):
        for name in dirs + names:
            path = Path(base) / name
            if path.is_symlink():
                raise ValueError("linked vendor member")
            if path.is_dir():
                if name == "__pycache__":
                    raise ValueError("vendor contains platform bytecode")
                continue
            if not path.is_file() or path.suffix.lower() in (".so", ".dylib", ".pyd", ".dll", ".a", ".o", ".pyc"):
                raise ValueError("vendor contains native, linked, or special artifact")
            relative = path.relative_to(vendor).as_posix()
            if not (relative.split("/")[0].endswith(".dist-info") or path.suffix == ".py"):
                raise ValueError("unexpected vendor package member")
            files[relative] = sha(path)
    digest = hashlib.sha256()
    # Match the reviewed VPS inventory's Path-component ordering, where
    # `iopath/` precedes `iopath-0.1.10.dist-info/`.
    for relative, value in sorted(files.items(), key=lambda row: Path(row[0])):
        digest.update(relative.encode() + b"\0" + bytes.fromhex(value))
    for folder, expected_name, expected_version in (
        ("iopath-0.1.10.dist-info", "iopath", "0.1.10"),
        ("antlr4_python3_runtime-4.9.3.dist-info", "antlr4-python3-runtime", "4.9.3"),
    ):
        metadata = (vendor / folder / "METADATA").read_text()
        if (f"Name: {expected_name}\n" not in metadata or
                f"Version: {expected_version}\n" not in metadata):
            raise ValueError("vendor metadata name/version differs")
    if digest.hexdigest() != VENDOR_SHA or len(files) != 78:
        raise ValueError("VPS pure-Python vendor inventory differs from reviewed digest")
    return {"inventory_sha256": digest.hexdigest(), "files_sha256": files}


def preflight(ai: Path = AI, source: Path = SOURCE, data: Path = DATA,
              mount: Path = MOUNT, root: Path = ROOT) -> dict:
    if sys.platform != "darwin" or platform.machine() != "arm64" or sys.version_info[:2] != (3, 11):
        raise RuntimeError("setup requires native arm64 macOS Python 3.11")
    if not mount.is_mount() or mount.is_symlink() or mount.stat().st_dev == root.stat().st_dev:
        raise ValueError("external mount missing or not distinct from workspace volume")
    for path in (ai, source, data):
        if path.exists() or path.is_symlink() or not path.resolve().is_relative_to(mount.resolve()):
            raise ValueError("setup paths must be fresh on external mount")
        if not path.parent.is_dir() or path.parent.is_symlink() or path.parent.stat().st_dev != mount.stat().st_dev:
            raise ValueError("setup parent is not on mounted external volume")
    if shutil.disk_usage(mount).free < RESERVE + CAP + BUFFER or shutil.disk_usage(root).free < RESERVE:
        raise ValueError("2 GiB setup cap + buffer or 10 GiB dual-disk floor unavailable")
    if sha(PACKAGE) != PACKAGE_SHA or sha(PROMPTS) != PROMPTS_SHA:
        raise ValueError("frozen local mustard package or prompt differs")
    package = json.loads(PACKAGE.read_text())
    photo_hashes = {Path(row["path"]).name: row["sha256"] for row in package["training_inputs"]}
    if any(name not in photo_hashes for name in NAMES):
        raise ValueError("selected parity photo is not a training input")
    lock = LOCK.read_text()
    if (sha(LOCK) != LOCK_SHA or len([row for row in lock.splitlines() if row.strip()]) != 18 or
            any(" --hash=sha256:" not in row for row in lock.splitlines())):
        raise ValueError("fixed 18-wheel hash lock changed")
    return {"schema": "sam21_m1_setup_v1", "status": "preflight", "ai": str(ai),
            "source": str(source), "data": str(data), "wheel_lock_sha256": sha(LOCK),
            "runner_sha256": sha(Path(__file__)),
            "package_sha256": PACKAGE_SHA, "prompt_sha256": PROMPTS_SHA,
            "selected_photo_sha256": {name: photo_hashes[name] for name in NAMES},
            "checkpoint_sha256": MODEL_SHA, "source_inventory_sha256": SOURCE_SHA,
            "max_total_bytes": CAP, "seconds": SECONDS, "min_free_each_bytes": RESERVE}


def bounded(command: list[str], label: str, ai: Path, source: Path, data: Path,
            deadline: float, report: dict) -> None:
    log = ai / f"{label}.log"
    env = os.environ.copy()
    env.update(TMPDIR=str(ai / "tmp"), TMP=str(ai / "tmp"), PIP_CACHE_DIR=str(ai / "pip-cache"),
               PYTHONNOUSERSITE="1", PYTHONDONTWRITEBYTECODE="1", SAM2_BUILD_CUDA="0",
               SSL_CERT_FILE="/etc/ssl/cert.pem", PIP_DISABLE_PIP_VERSION_CHECK="1")
    process = None
    failure = None
    started = time.monotonic()
    try:
        with log.open("wb") as output:
            process = subprocess.Popen(command, cwd=ai, env=env, stdout=output,
                                       stderr=subprocess.STDOUT, start_new_session=True)
            while process.poll() is None:
                if time.monotonic() > deadline:
                    failure = "600-second total deadline"
                elif (shutil.disk_usage(MOUNT).free < RESERVE or
                      shutil.disk_usage(ROOT).free < RESERVE):
                    failure = "10 GiB dual-disk floor"
                elif bytes_under(ai) + bytes_under(source) + bytes_under(data) > CAP:
                    failure = "2 GiB combined setup cap"
                elif log.stat().st_size > LOG_CAP:
                    failure = "2 MiB stage log cap"
                if failure:
                    break
                time.sleep(0.5)
    finally:
        if process is not None and process.poll() is None:
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.wait()
    if (failure is None and (process is None or process.returncode or time.monotonic() > deadline or
            log.stat().st_size > LOG_CAP or
            bytes_under(ai) + bytes_under(source) + bytes_under(data) > CAP or
            min(shutil.disk_usage(MOUNT).free, shutil.disk_usage(ROOT).free) < RESERVE)):
        failure = "stage exit or postflight resource gate"
    report["stages"].append({"name": label, "command": command, "seconds": round(time.monotonic()-started, 3),
                             "exit_code": None if process is None else process.returncode,
                             "log": str(log), "failure": failure})
    if failure:
        raise RuntimeError(f"{label}: {failure}")


def commands(ai: Path, source: Path, data: Path) -> list[tuple[str, list[str]]]:
    def remote(path: str) -> str:
        return "vps:" + path
    result = [
        ("source-code", ["rsync", "-a", "--include=*/", "--include=*.py", "--include=*.yaml",
                         "--exclude=*", remote(PREREQ + "/source/sam2/"), str(source / "sam2/")]),
        ("source-license", ["rsync", "-a", remote(PREREQ + "/source/LICENSE"), str(source / "LICENSE")]),
        ("checkpoint", ["rsync", "-a", remote(PREREQ + "/sam2.1_hiera_tiny.pt"),
                         str(ai / "checkpoints/sam2.1_hiera_tiny.pt")]),
    ]
    for item in VENDOR_DIRS:
        result.append(("vendor-" + item.replace(".", "-"),
                       ["rsync", "-a", "--exclude=__pycache__/", "--exclude=*.pyc",
                        remote(PREREQ + "/deps/" + item + "/"), str(ai / "vendor-pure" / item) + "/"]))
    for name in NAMES:
        result.append(("photo-" + name[:-4], ["rsync", "-a", remote(PHOTOS + "/" + name),
                                             str(data / "photos" / name)]))
    for index in (0, 1):
        batch = PRIOR + f"/batch-{index}"
        target = data / "prior" / f"batch-{index}"
        result.append((f"prior-{index}", ["rsync", "-a", "--include=*/", "--include=manifest.json",
                                           "--include=*.png", "--exclude=*", remote(batch + "/"), str(target) + "/"]))
    result.append(("prior-parent", ["rsync", "-a", remote(PRIOR + "/manifest.json"),
                                    str(data / "prior/manifest.json")]))
    result.append(("venv", [sys.executable, "-m", "venv", "--copies", str(ai / "venv")]))
    result.append(("wheel-install", [str(ai / "venv/bin/python"), "-m", "pip", "install", "--no-input",
                                      "--no-deps", "--only-binary=:all:", "--require-hashes",
                                      "--cache-dir", str(ai / "pip-cache"), "-r", str(LOCK)]))
    result.append(("import-probe", ["/usr/bin/env",
                                    "PYTHONPATH=" + str(source),
                                    str(ai / "venv/bin/python"), "-c",
                                    "import torch, torchvision, hydra, omegaconf, iopath, antlr4; "
                                    "from sam2.build_sam import build_sam2; "
                                    "from sam2.sam2_image_predictor import SAM2ImagePredictor; "
                                    "assert torch.__version__.split('+')[0] == '2.7.0'; "
                                    "assert torchvision.__version__.split('+')[0] == '0.22.0'; "
                                    "print('imports_ok', torch.backends.mps.is_available())"]))
    return result


def validate_inputs(plan: dict, ai: Path, source: Path, data: Path) -> dict:
    if sha(LOCK) != LOCK_SHA:
        raise ValueError("wheel lock changed")
    if source_digest(source) != SOURCE_SHA:
        raise ValueError("selected SAM2 code/config/license differs from pinned VPS inventory")
    checkpoint = ai / "checkpoints/sam2.1_hiera_tiny.pt"
    if checkpoint.is_symlink() or checkpoint.stat().st_size != 156_008_466 or sha(checkpoint) != MODEL_SHA:
        raise ValueError("checkpoint size/hash differs")
    for name, expected in plan["selected_photo_sha256"].items():
        path = data / "photos" / name
        if path.is_symlink() or sha(path) != expected:
            raise ValueError(f"training photo differs: {name}")
    if sha(data / "package.json") != PACKAGE_SHA or sha(data / "prompts.json") != PROMPTS_SHA:
        raise ValueError("local package or prompt copy differs")
    parent = data / "prior/manifest.json"
    if sha(parent) != PRIOR_SHA:
        raise ValueError("prior parent manifest differs")
    for index, expected in enumerate(BATCH_SHA):
        batch = data / "prior" / f"batch-{index}"
        if sha(batch / "manifest.json") != expected:
            raise ValueError("prior batch manifest differs")
        rows = json.loads((batch / "manifest.json").read_text())["images"]
        if len(rows) != 8:
            raise ValueError("prior batch does not contain exactly eight sealed masks")
        for row in rows:
            for kind in ("raw", "cleaned"):
                path = batch / f"{kind}_masks" / (row["name"] + ".png")
                if path.is_symlink() or sha(path) != row[f"{kind}_mask_sha256"]:
                    raise ValueError("prior reference mask hash differs")
    artifacts = {"vendor": vendor_inventory(ai / "vendor-pure"), "source_sha256": SOURCE_SHA,
                 "checkpoint_sha256": MODEL_SHA}
    hook = ai / "venv/lib/python3.11/site-packages/sam21-reviewed-vendor.pth"
    if hook.exists() or hook.is_symlink():
        if hook.is_symlink() or hook.read_text() != str(ai / "vendor-pure") + "\n":
            raise ValueError("reviewed vendor path hook differs")
        artifacts["vendor_path_hook_sha256"] = sha(hook)
    return artifacts


def execute(ai: Path = AI, source: Path = SOURCE, data: Path = DATA) -> dict:
    plan = preflight(ai, source, data)
    ai.mkdir()
    (ai / "tmp").mkdir()
    (ai / "pip-cache").mkdir()
    (ai / "checkpoints").mkdir()
    (ai / "vendor-pure").mkdir()
    source.mkdir()
    (source / "sam2").mkdir()
    data.mkdir()
    (data / "photos").mkdir()
    (data / "prior").mkdir()
    for index in (0, 1):
        (data / "prior" / f"batch-{index}").mkdir()
    shutil.copy2(PACKAGE, data / "package.json")
    shutil.copy2(PROMPTS, data / "prompts.json")
    plan["status"] = "running"
    plan["stages"] = []
    deadline = time.monotonic() + SECONDS
    try:
        for label, command in commands(ai, source, data):
            if label == "venv":
                plan["artifacts"] = validate_inputs(plan, ai, source, data)
            if label == "import-probe":
                hook = ai / "venv/lib/python3.11/site-packages/sam21-reviewed-vendor.pth"
                if hook.exists() or hook.is_symlink() or not hook.parent.is_dir():
                    raise ValueError("vendor path hook must be fresh inside isolated venv")
                hook.write_text(str(ai / "vendor-pure") + "\n")
                plan["artifacts"]["vendor_path_hook_sha256"] = sha(hook)
            bounded(command, label, ai, source, data, deadline, plan)
        plan["artifacts"] = validate_inputs(plan, ai, source, data)
        plan["total_bytes"] = bytes_under(ai) + bytes_under(source) + bytes_under(data)
        if (plan["total_bytes"] > CAP or min(shutil.disk_usage(MOUNT).free, shutil.disk_usage(ROOT).free) < RESERVE or
                sha(Path(__file__)) != plan["runner_sha256"] or sha(LOCK) != plan["wheel_lock_sha256"]):
            raise ValueError("setup postflight cap or dual-disk reserve failed")
        plan["status"] = "complete"
    except BaseException as error:
        plan["status"] = "failed"
        plan["failure"] = f"{type(error).__name__}: {error}"
        (ai / "setup-report.json").write_text(json.dumps(plan, indent=2, sort_keys=True) + "\n")
        raise
    (ai / "setup-report.json").write_text(json.dumps(plan, indent=2, sort_keys=True) + "\n")
    return plan


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--execute", action="store_true", help="perform approved fixed setup; no inference")
    args = parser.parse_args()
    result = execute() if args.execute else preflight()
    print(json.dumps({k: result[k] for k in ("status", "ai", "source", "data", "wheel_lock_sha256")}, indent=2))


if __name__ == "__main__":
    main()
