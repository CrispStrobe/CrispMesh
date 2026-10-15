#!/usr/bin/env python3
"""Fetch two ETH3D pipes archives and inventory them without extraction."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import time
from datetime import datetime, timezone
import urllib.error
import urllib.request


REPO = Path(__file__).resolve().parents[2]
DEST = REPO / ".local-tools/test-data/eth3d-multiview-pipes-v1"
ORIGIN = "https://www.eth3d.net"
FILES = {
    "pipes_dslr_undistorted.7z": 145_321_540,
    "pipes_dslr_scan_eval.7z": 53_851_871,
}
MAX_FILE_BYTES = 200 * 1024**2
MAX_TOTAL_BYTES = 250 * 1024**2
MIN_FREE_BYTES = 10 * 1024**3
WORKSPACE_RESERVE = 500 * 1024**2
MAX_FILE_SECONDS = 120
MAX_RUN_SECONDS = 300
MAX_MEMBERS = 10_000
MAX_EXPANDED_BYTES = 4 * 1024**3
CHUNK = 1024 * 1024
PAPER_AUTHORS = "Thomas Schöps, Johannes L. Schönberger, Silvano Galliani, Torsten Sattler, Konrad Schindler, Marc Pollefeys, Andreas Geiger"
PIPES_V2_ORIGINAL_MANIFEST_SHA256 = "84f09c185f0333b8747c078c2b605b6bda9db6268d5b0c86464b205afc2d4149"


class SameOriginRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, msg, headers, newurl):
        from urllib.parse import urlsplit

        parsed = urlsplit(newurl)
        if parsed.scheme != "https" or parsed.netloc != "www.eth3d.net":
            raise ValueError(f"unexpected download redirect: {newurl}")
        return super().redirect_request(request, fp, code, msg, headers, newurl)


def safe_member(name: str) -> None:
    path = PurePosixPath(name)
    if (not name or "\\" in name or "\x00" in name or path.is_absolute()
            or name.startswith("./") or "//" in name
            or any(part in (".", "..") for part in name.split("/"))):
        raise ValueError(f"unsafe archive member: {name!r}")


def parse_7z_listing(output: str) -> dict:
    """Parse `7z l -slt`; reject paths and links before extraction is considered."""
    lines = output.splitlines()
    try:
        start = next(i for i, line in enumerate(lines) if line.startswith("----------")) + 1
    except StopIteration as exc:
        raise ValueError("7z listing has no member section") from exc
    entries = []
    fields = {}
    for line in [*lines[start:], ""]:
        if not line:
            if fields:
                entries.append(fields)
                fields = {}
            continue
        if " = " in line:
            key, value = line.split(" = ", 1)
            fields[key] = value
    if not entries or len(entries) > MAX_MEMBERS:
        raise ValueError("archive member count outside limit")
    seen = set()
    total = 0
    members = []
    for entry in entries:
        name = entry.get("Path", "")
        safe_member(name)
        if name in seen:
            raise ValueError(f"duplicate archive member: {name}")
        seen.add(name)
        attributes = entry.get("Attributes", "")
        kinds = attributes.split()
        if entry.get("Encrypted") != "-":
            raise ValueError(f"encrypted or unknown archive entry: {name}")
        if len(kinds) != 2 or kinds[0] not in ("A_", "D_"):
            raise ValueError(f"unsupported archive entry attributes: {name}: {attributes}")
        directory = kinds[0] == "D_"
        if ((directory and not kinds[1].startswith("d")) or
                (not directory and not kinds[1].startswith("-")) or
                entry.get("Symbolic Link") or entry.get("Hard Link")):
            raise ValueError(f"archive link or special entry is not allowed: {name}")
        try:
            size = int(entry["Size"])
        except (KeyError, ValueError) as exc:
            raise ValueError(f"missing or invalid member size: {name}") from exc
        if size < 0:
            raise ValueError(f"negative member size: {name}")
        if directory and size:
            raise ValueError(f"directory has nonzero size: {name}")
        total += size
        if total > MAX_EXPANDED_BYTES:
            raise ValueError("archive expanded size exceeds limit")
        members.append({"path": name, "size_bytes": size,
                        "directory": directory})
    return {"member_count": len(members), "expanded_bytes": total, "members": members}


def inventory(path: Path, deadline: float) -> dict:
    binary = shutil.which("7z")
    if not binary:
        raise RuntimeError("7z is required for archive inventory")
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise TimeoutError("overall time limit exceeded before inventory")
    result = subprocess.run([binary, "l", "-slt", str(path)], capture_output=True,
                            text=True, timeout=min(30, remaining), check=False)
    if result.returncode:
        raise ValueError(f"7z could not list {path.name}: {result.stderr[-500:]}")
    return parse_7z_listing(result.stdout)


def fetch_one(root: Path, name: str, size: int, opener, deadline: float) -> dict:
    if name not in FILES or size != FILES[name]:
        raise ValueError("unapproved source file")
    if size > MAX_FILE_BYTES or time.monotonic() >= deadline:
        raise ValueError("source exceeds budget or time limit")
    target = root / name
    partial = root / (name + ".partial")
    if target.exists() or partial.exists():
        raise FileExistsError(f"refusing to replace existing download: {name}")
    request = urllib.request.Request(f"{ORIGIN}/data/{name}",
                                     headers={"User-Agent": "crispmesh-eth3d-reference/1"})
    digest = hashlib.sha256()
    count = 0
    started = time.monotonic()
    with opener.open(request, timeout=20) as response, partial.open("xb") as sink:
        if response.geturl() != request.full_url:
            raise ValueError("download URL changed")
        length = response.headers.get("Content-Length")
        if length is None or int(length) != size:
            raise ValueError(f"Content-Length mismatch for {name}: {length}")
        while True:
            if time.monotonic() - started > MAX_FILE_SECONDS or time.monotonic() > deadline:
                raise TimeoutError(f"download timeout: {name}")
            if shutil.disk_usage(root).free < MIN_FREE_BYTES + WORKSPACE_RESERVE:
                raise OSError("free disk space fell below reserve during download")
            block = response.read(min(CHUNK, size - count + 1))
            if not block:
                break
            count += len(block)
            if count > size or count > MAX_FILE_BYTES:
                raise ValueError(f"download exceeds declared size: {name}")
            sink.write(block)
            digest.update(block)
    if count != size:
        raise ValueError(f"short download: {name}: {count} of {size}; partial preserved")
    partial.rename(target)
    return {"filename": name, "url": request.full_url, "bytes": count,
            "sha256_measured": digest.hexdigest(), "upstream_sha256_published": False}


def run(root: Path = DEST, opener=None) -> dict:
    if root.exists() or root.is_symlink():
        raise FileExistsError(f"destination must be fresh: {root}")
    if any(parent.is_symlink() for parent in root.parents):
        raise ValueError("destination has a symlinked parent")
    if sum(FILES.values()) > MAX_TOTAL_BYTES:
        raise ValueError("approved sources exceed aggregate budget")
    if shutil.disk_usage(root.parent).free < MIN_FREE_BYTES + WORKSPACE_RESERVE:
        raise OSError("insufficient free disk space for 10 GiB floor and 500 MiB reserve")
    if not shutil.which("7z"):
        raise RuntimeError("7z is required before fetching")
    root.mkdir(parents=True, exist_ok=False)
    opener = opener or urllib.request.build_opener(SameOriginRedirect())
    deadline = time.monotonic() + MAX_RUN_SECONDS
    seven_zip = Path(shutil.which("7z"))
    version = subprocess.run([str(seven_zip)], capture_output=True, text=True,
                             timeout=5, check=False).stdout.splitlines()
    records = []
    for name, size in FILES.items():
        if shutil.disk_usage(root).free < MIN_FREE_BYTES + WORKSPACE_RESERVE:
            raise OSError("free disk space fell below reserve")
        file_record = fetch_one(root, name, size, opener, deadline)
        file_record["inventory"] = inventory(root / name, deadline)
        records.append(file_record)
    manifest = {
        "schema_version": 1,
        "dataset": "ETH3D high-resolution multi-view, pipes training scene",
        "source": "https://www.eth3d.net/datasets",
        "documentation": "https://www.eth3d.net/documentation",
        "paper": "https://www.eth3d.net/data/schoeps2017cvpr.pdf",
        "paper_authors": PAPER_AUTHORS,
        "paper_title": "A Multi-View Stereo Benchmark with High-Resolution Images and Multi-Camera Videos",
        "license": "CC BY-NC-SA 4.0; research use; commercial use requires review",
        "stage": "downloaded and inventoried; not extracted or evaluated",
        "fetched_utc": datetime.now(timezone.utc).isoformat(),
        "tool": {"script": str(Path(__file__).relative_to(REPO)),
                 "sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                 "seven_zip_path": str(seven_zip),
                 "seven_zip_sha256": hashlib.sha256(seven_zip.read_bytes()).hexdigest(),
                 "seven_zip_version": version[1] if len(version) > 1 else "unknown"},
        "archives": records,
        "total_download_bytes": sum(record["bytes"] for record in records),
        "total_declared_expanded_bytes": sum(record["inventory"]["expanded_bytes"] for record in records),
    }
    (root / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    return manifest


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(CHUNK), b""):
            digest.update(block)
    return digest.hexdigest()


def review_inventory(root: Path, expected_manifest_sha256: str = PIPES_V2_ORIGINAL_MANIFEST_SHA256) -> dict:
    """Recheck the saved bytes and write a separate, corrected offline inventory."""
    if root.is_symlink() or any(parent.is_symlink() for parent in root.parents):
        raise ValueError("inventory root may not contain symlinks")
    manifest_path = root / "manifest.json"
    output_path = root / "inventory-reviewed.json"
    if output_path.exists() or output_path.is_symlink():
        raise FileExistsError(f"refusing to replace reviewed inventory: {output_path}")
    original_hash = sha256_file(manifest_path)
    if original_hash != expected_manifest_sha256:
        raise ValueError("original download manifest differs from reviewed SHA-256")
    manifest = json.loads(manifest_path.read_text())
    originals = manifest.get("archives")
    if (not isinstance(originals, list) or len(originals) != len(FILES) or
            {entry.get("filename") for entry in originals} != set(FILES)):
        raise ValueError("original manifest does not list the approved archives")
    deadline = time.monotonic() + 60
    records = []
    for old in originals:
        name = old["filename"]
        path = root / name
        if path.is_symlink() or not path.is_file():
            raise ValueError(f"missing or linked archive: {name}")
        if path.stat().st_size != FILES[name] or old.get("bytes") != FILES[name]:
            raise ValueError(f"archive byte count changed: {name}")
        measured = sha256_file(path)
        if measured != old.get("sha256_measured"):
            raise ValueError(f"archive digest changed: {name}")
        records.append({"filename": name, "bytes": FILES[name],
                        "sha256": measured, "inventory": inventory(path, deadline)})
    reviewed = {
        "schema_version": 1,
        "stage": "offline inventory reviewed; no extraction",
        "reviewed_utc": datetime.now(timezone.utc).isoformat(),
        "original_manifest": manifest_path.name,
        "original_manifest_sha256": original_hash,
        "review_script_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "archives": records,
        "total_expanded_bytes": sum(item["inventory"]["expanded_bytes"] for item in records),
    }
    with output_path.open("x") as stream:
        json.dump(reviewed, stream, indent=2)
        stream.write("\n")
    return reviewed


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inventory-only", type=Path, metavar="EXISTING_DIRECTORY")
    args = parser.parse_args()
    if args.inventory_only is not None:
        print(json.dumps(review_inventory(args.inventory_only), indent=2))
    else:
        print(json.dumps(run(), indent=2))
