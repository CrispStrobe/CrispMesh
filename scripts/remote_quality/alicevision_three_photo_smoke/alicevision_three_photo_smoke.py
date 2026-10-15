"""Private, bounded CPU sparse smoke on three sealed bunny photographs."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import selectors
import shutil
import signal
import subprocess
import tarfile
import time
import urllib.request


RELEASE_URL = "https://github.com/alicevision/AliceVision/releases/download/v3.3.0/AliceVision-3.3.0-Linux.tar.gz"
RELEASE_SHA256 = "f43f498312859af627f2f7f65a6d33c2a3411b37989b8b680c04c8c690dcb640"
ARCHIVE_BYTES = 1_505_191_867
MAX_EXPANDED = 8 << 30


def short_command(argv: list[str], timeout: int = 20, env: dict[str, str] | None = None,
                  required_tokens: tuple[str, ...] = ()) -> dict:
    start = time.monotonic()
    process = None
    try:
        process = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                   start_new_session=True, env=env)
        assert process.stdout is not None
        output = bytearray()
        deadline = start + timeout
        with selectors.DefaultSelector() as selector:
            selector.register(process.stdout, selectors.EVENT_READ)
            while selector.get_map():
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    return {"timed_out": True, "seconds": round(time.monotonic() - start, 3)}
                for key, _ in selector.select(remaining):
                    block = os.read(key.fd, min(4096, 65_537 - len(output)))
                    if not block:
                        selector.unregister(key.fileobj)
                    else:
                        output.extend(block)
                        if len(output) > 65_536:
                            return {"output_capped": True, "seconds": round(time.monotonic() - start, 3)}
        process.wait(timeout=max(0.1, deadline - time.monotonic()))
        return {"returncode": process.returncode,
                "seconds": round(time.monotonic() - start, 3),
                "missing_libraries": b"not found" in output,
                "required_tokens_present": {token: token.encode() in output
                                            for token in required_tokens},
                "output_tail": output[-4096:].decode("utf-8", "replace")}
    except subprocess.TimeoutExpired:
        return {"timed_out": True, "seconds": round(time.monotonic() - start, 3)}
    except OSError as exc:
        return {"unavailable": True, "error": f"{type(exc).__name__}: {exc}"[:256],
                "seconds": round(time.monotonic() - start, 3)}
    finally:
        if process is not None:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=2)
            if process.stdout is not None:
                process.stdout.close()


def download(path: Path, deadline: float) -> dict:
    digest = hashlib.sha256()
    total = 0
    with urllib.request.urlopen(RELEASE_URL, timeout=30) as source, path.open("xb") as target:
        while True:
            if time.monotonic() > deadline:
                raise TimeoutError("archive download exceeded deadline")
            block = source.read(1 << 20)
            if not block:
                break
            total += len(block)
            if total > ARCHIVE_BYTES:
                raise ValueError("archive exceeds pinned release size")
            digest.update(block)
            target.write(block)
    if total != ARCHIVE_BYTES or digest.hexdigest() != RELEASE_SHA256:
        raise ValueError("official release asset size or SHA-256 differs")
    return {"bytes": total, "sha256": digest.hexdigest()}


def inspect_tar(path: Path) -> dict:
    files = 0
    expanded = 0
    with tarfile.open(path, "r:gz") as archive:
        for item in archive:
            name = PurePosixPath(item.name)
            if (name.is_absolute() or ".." in name.parts or
                    not (item.isfile() or item.isdir() or item.issym() or item.islnk())):
                raise ValueError("archive has unsafe member")
            if item.issym() or item.islnk():
                target = PurePosixPath(item.linkname)
                if target.is_absolute() or ".." in target.parts:
                    raise ValueError("archive has unsafe link")
            files += 1
            expanded += item.size if item.isfile() else 0
            if files > 100_000 or expanded > MAX_EXPANDED:
                raise ValueError("archive exceeds member or expanded-byte cap")
    return {"members": files, "expanded_regular_bytes": expanded}


def bundled_runtime_paths(extracted: Path) -> tuple[Path, Path]:
    lib_dirs = {path.parent for path in extracted.rglob("libaliceVision_cmdline.so.3")
                if path.is_file()}
    if len(lib_dirs) != 1:
        raise RuntimeError("bundled AliceVision library directory missing or ambiguous")
    lib_dir = lib_dirs.pop()
    install_root = lib_dir.parent
    if not (install_root / "share" / "aliceVision" / "config.ocio").is_file():
        raise RuntimeError("bundled AliceVision OCIO configuration missing")
    return lib_dir, install_root


class binary:
    """Namespace retained for the reviewed loader helpers, now self-contained."""

    SHA256 = RELEASE_SHA256
    ARCHIVE_BYTES = ARCHIVE_BYTES
    MAX_EXPANDED = MAX_EXPANDED
    short_command = staticmethod(short_command)
    download = staticmethod(download)
    inspect_tar = staticmethod(inspect_tar)
    bundled_runtime_paths = staticmethod(bundled_runtime_paths)


DATASET_CANDIDATES = (
    Path("/kaggle/input/crispmesh-bunny-3photo-research-smoke"),
    Path("/kaggle/input/datasets/chr1str/crispmesh-bunny-3photo-research-smoke"),
)
MANIFEST_SHA256 = "e36fcbc214d76d3c29658c1b1595e4d6d3b2f0318c6523096de232d5e37483b9"
PHOTOS = {
    "frame_0000.png": (1_810_461, "c40614be25fd5f2d9113bbb8e64156598b1fc7c59f66bd4542d837ea3488c9e7"),
    "frame_0022.png": (1_713_161, "82c50e9fb350ae7f1324398d0fff30271fa66438fea7d585fa2904c8ce1087af"),
    "frame_0044.png": (1_682_651, "16c67b9419c4f3e8a219be02d45c9e9d18595a6fd73d9f2c19205edfb86b4543"),
}
SCRATCH = Path("/tmp/alicevision-three-photo-smoke")
OUTPUT = Path("/kaggle/working/alicevision-three-photo-smoke.json")
MAX_SECONDS = 20 * 60
MAX_WORK_BYTES = 1 << 30
MIN_FREE_AFTER = 4 << 30


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def resolve_dataset(candidates: tuple[Path, ...] = DATASET_CANDIDATES) -> Path:
    present = [path for path in candidates if path.is_dir()]
    if len(present) != 1:
        raise RuntimeError("private dataset mount missing or ambiguous")
    return present[0]


def verify_inputs(dataset: Path) -> dict:
    if sha256(dataset / "selection-manifest.json") != MANIFEST_SHA256:
        raise ValueError("private dataset or sealed selection manifest mismatch")
    manifest = json.loads((dataset / "selection-manifest.json").read_text())
    if {item["filename"] for item in manifest["photos"]} != set(PHOTOS):
        raise ValueError("selection manifest photograph set differs")
    if {path.name for path in dataset.glob("*.png")} != set(PHOTOS):
        raise ValueError("dataset PNG set differs")
    for filename, (size, expected_sha) in PHOTOS.items():
        path = dataset / filename
        if path.stat().st_size != size or sha256(path) != expected_sha:
            raise ValueError(f"sealed photograph mismatch: {filename}")
    return {name: {"bytes": size, "sha256": digest}
            for name, (size, digest) in PHOTOS.items()}


def work_bytes(path: Path) -> int:
    return sum(item.stat().st_size for item in path.rglob("*") if item.is_file())


def run_stage(name: str, executable: Path, arguments: list[str], timeout: int,
              env: dict[str, str], work: Path, report: dict) -> None:
    if shutil.disk_usage(SCRATCH).free < MIN_FREE_AFTER:
        raise RuntimeError("free-space floor breached before stage")
    result = binary.short_command([str(executable), *arguments], timeout, env=env)
    report["stages"].append({"name": name, "argv": [executable.name, *arguments],
                             "result": result, "work_bytes": work_bytes(work)})
    if report["stages"][-1]["work_bytes"] > MAX_WORK_BYTES:
        raise RuntimeError(f"one-GiB work cap exceeded at {name}")
    if shutil.disk_usage(SCRATCH).free < MIN_FREE_AFTER:
        raise RuntimeError(f"free-space floor breached after {name}")
    if result.get("returncode") != 0:
        raise RuntimeError(f"{name} failed or exceeded command output/time cap")


def main() -> None:
    start = time.monotonic()
    signal.signal(signal.SIGALRM, lambda *_: (_ for _ in ()).throw(TimeoutError("20-minute deadline")))
    signal.alarm(MAX_SECONDS)
    report: dict = {"schema": "alicevision_three_photo_cpu_sparse_smoke_v1",
                    "status": "running", "dataset_ref": "chr1str/crispmesh-bunny-3photo-research-smoke",
                    "dataset_manifest_sha256": MANIFEST_SHA256,
                    "official_release_sha256": binary.SHA256,
                    "gpu_enabled": False, "scanner_or_supplied_poses_used": False,
                    "stages": []}
    try:
        if SCRATCH.exists() or OUTPUT.exists():
            raise FileExistsError("fresh scratch/output required")
        if not SCRATCH.parent.is_dir() or not OUTPUT.parent.is_dir():
            raise FileNotFoundError("Kaggle scratch/output directories missing")
        dataset = resolve_dataset()
        report["dataset_mount"] = str(dataset)
        report["photos"] = verify_inputs(dataset)
        report["starting_free_bytes"] = shutil.disk_usage(SCRATCH.parent).free
        if report["starting_free_bytes"] < (binary.ARCHIVE_BYTES + binary.MAX_EXPANDED +
                                            MAX_WORK_BYTES + MIN_FREE_AFTER):
            raise RuntimeError("insufficient disk for archive, extracted files, work and floor")
        SCRATCH.mkdir()
        archive = SCRATCH / "alicevision.tar.gz"
        report["archive"] = binary.download(archive, start + 8 * 60)
        report["tar_inventory"] = binary.inspect_tar(archive)
        extracted = SCRATCH / "release"
        extracted.mkdir()
        extraction = binary.short_command(["tar", "-xzf", str(archive), "-C", str(extracted)], 5 * 60)
        report["extract"] = {key: value for key, value in extraction.items() if key != "output_tail"}
        if extraction.get("returncode") != 0:
            raise RuntimeError("pinned archive extraction failed or exceeded cap")
        lib_dir, install_root = binary.bundled_runtime_paths(extracted)
        sensor_db = install_root / "share" / "aliceVision" / "cameraSensors.db"
        if not sensor_db.is_file():
            raise RuntimeError("bundled camera sensor database missing")
        report["bundled_install_root"] = str(install_root.relative_to(extracted))
        env = os.environ.copy()
        env["LD_LIBRARY_PATH"] = str(lib_dir) + (
            ":" + env["LD_LIBRARY_PATH"] if env.get("LD_LIBRARY_PATH") else "")
        env["ALICEVISION_ROOT"] = str(install_root)
        commands = {
            "cameraInit": ("aliceVision_cameraInit", ("--imageFolder", "--sensorDatabase", "--defaultFieldOfView", "--output")),
            "featureExtraction": ("aliceVision_featureExtraction", ("--input", "--describerTypes", "--forceCpuExtraction", "--output")),
            "imageMatching": ("aliceVision_imageMatching", ("--input", "--featuresFolders", "--minNbImages", "--output")),
            "featureMatching": ("aliceVision_featureMatching", ("--input", "--featuresFolders", "--imagePairsList", "--describerTypes", "--output")),
            "incrementalSfM": ("aliceVision_incrementalSfM", ("--input", "--featuresFolders", "--matchesFolders", "--describerTypes", "--output", "--outputViewsAndPoses", "--extraInfoFolder")),
        }
        binaries = {}
        report["help_checks"] = {}
        for stage, (name, flags) in commands.items():
            executable = install_root / "bin" / name
            if not executable.is_file():
                raise RuntimeError(f"expected pinned executable absent: {name}")
            binaries[stage] = executable
            check = binary.short_command([str(executable), "--help"], 20, env=env,
                                         required_tokens=flags)
            report["help_checks"][stage] = {key: value for key, value in check.items()
                                            if key != "output_tail"}
            if (check.get("returncode") not in (0, 1) or
                    not all(check.get("required_tokens_present", {}).values()) or
                    check.get("missing_libraries")):
                raise RuntimeError(f"pinned CLI flag/help validation failed: {stage}")
        work = SCRATCH / "work"
        work.mkdir()
        features = work / "features"
        matches = work / "matches"
        extra = work / "sfm_extra"
        for directory in (features, matches, extra):
            directory.mkdir()
        camera = work / "cameraInit.sfm"
        pairs = work / "imagePairs.txt"
        sfm = work / "sfm.sfm"
        poses = work / "cameras.sfm"
        run_stage("cameraInit", binaries["cameraInit"],
                  ["--imageFolder", str(dataset), "--sensorDatabase", str(sensor_db),
                   "--defaultFieldOfView", "45", "--output", str(camera)],
                  90, env, work, report)
        camera_data = json.loads(camera.read_text())
        report["camera_init_views"] = len(camera_data.get("views", []))
        if report["camera_init_views"] != 3:
            raise RuntimeError("cameraInit did not import exactly three views")
        run_stage("featureExtraction", binaries["featureExtraction"],
                  ["--input", str(camera), "--describerTypes", "sift",
                   "--forceCpuExtraction", "true", "--output", str(features)],
                  180, env, work, report)
        run_stage("imageMatching", binaries["imageMatching"],
                  ["--input", str(camera), "--featuresFolders", str(features),
                   "--minNbImages", "200", "--output", str(pairs)],
                  60, env, work, report)
        if not pairs.is_file() or pairs.stat().st_size == 0:
            raise RuntimeError("imageMatching produced no pair list")
        run_stage("featureMatching", binaries["featureMatching"],
                  ["--input", str(camera), "--featuresFolders", str(features),
                   "--imagePairsList", str(pairs), "--describerTypes", "sift",
                   "--output", str(matches)],
                  120, env, work, report)
        if not any(path.is_file() and path.stat().st_size for path in matches.rglob("*")):
            raise RuntimeError("featureMatching produced no nonempty file")
        run_stage("incrementalSfM", binaries["incrementalSfM"],
                  ["--input", str(camera), "--featuresFolders", str(features),
                   "--matchesFolders", str(matches), "--describerTypes", "sift",
                   "--output", str(sfm), "--outputViewsAndPoses", str(poses),
                   "--extraInfoFolder", str(extra)],
                  180, env, work, report)
        report["sfm_bytes"] = sfm.stat().st_size if sfm.is_file() else 0
        report["pose_file_bytes"] = poses.stat().st_size if poses.is_file() else 0
        if not report["sfm_bytes"] or not report["pose_file_bytes"]:
            raise RuntimeError("SfM did not produce nonempty model and camera files")
        report["status"] = "cpu_sparse_smoke_complete"
    except Exception as exc:
        report["status"] = "failed"
        report["error"] = f"{type(exc).__name__}: {exc}"[:512]
    finally:
        signal.alarm(0)
    report["elapsed_seconds"] = round(time.monotonic() - start, 3)
    report["ending_work_bytes"] = work_bytes(SCRATCH / "work") if (SCRATCH / "work").is_dir() else 0
    payload = json.dumps(report, sort_keys=True, indent=2) + "\n"
    if len(payload.encode()) > 1 << 20:
        raise RuntimeError("durable receipt exceeds one MiB")
    with OUTPUT.open("x") as stream:
        stream.write(payload)
    print(json.dumps({"status": report["status"], "receipt": str(OUTPUT)}))


if __name__ == "__main__":
    main()
