"""Verified, allowlisted relocation of completed reconstruction runs.

No move happens on import. Run with --run NAME after reviewing the preflight.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat

WORKSPACE = Path(__file__).resolve().parents[2]
MOUNT = Path("/Volumes/backups")
DEST_ROOT = MOUNT / "code/crispmesh-data/build-opencv"
ALLOWLIST = frozenset({
    "classical-ycb-native-masked-009",
    "classical-ycb-native-masked-011-continuation",
    "classical-ycb-calibrated-012",
    "classical-bunny-retry-dense-002",
    "classical-bunny-native-masked-005",
})
MIN_EXTERNAL_FREE = 10 * 1024**3
COPY_BUFFER = 256 * 1024**2


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def sync_open_mode(platform_name: str) -> str:
    """Windows CRT needs a writable fd for fsync; POSIX accepts read-only."""
    return "r+b" if platform_name == "nt" else "rb"


def sync_files(root: Path, relative_names: list[str]) -> None:
    """Flush only the copied files, not unrelated concurrent filesystem jobs."""
    for name in relative_names:
        # Windows' CRT rejects fsync on a read-only descriptor (EBADF), even
        # though Linux/macOS accept it. Windows r+b performs no write; the
        # copied bytes are rehashed after the flush.
        with (root / name).open(sync_open_mode(os.name)) as stream:
            os.fsync(stream.fileno())


def link_points_to_destination(link: Path, destination: Path) -> bool:
    """Compare resolved targets; Windows may spell readlink() with a device prefix."""
    if not link.is_symlink():
        return False
    try:
        return link.resolve(strict=False) == destination.resolve(strict=False)
    except (OSError, RuntimeError):
        return False


def inventory(root: Path) -> dict:
    """Full relative file hashes plus empty-directory inventory; reject links/devices."""
    if root.is_symlink() or not root.is_dir():
        raise ValueError(f"not a real directory: {root}")
    files: dict[str, dict] = {}
    dirs: list[str] = []
    for base, subdirs, names in os.walk(root, followlinks=False):
        base_path = Path(base)
        for name in sorted(subdirs):
            path = base_path / name
            if not stat.S_ISDIR(path.lstat().st_mode):
                raise ValueError(f"symlink or special directory: {path}")
            dirs.append(path.relative_to(root).as_posix())
        for name in sorted(names):
            path = base_path / name
            if not stat.S_ISREG(path.lstat().st_mode):
                raise ValueError(f"symlink or special file: {path}")
            files[path.relative_to(root).as_posix()] = {
                "bytes": path.stat().st_size,
                "sha256": sha256(path),
            }
    return {"directories": sorted(dirs), "files": dict(sorted(files.items()))}


def verify_external_mount(mount: Path, workspace: Path, destination_root: Path,
                          *, expected_device: int | None = None) -> int:
    if not mount.is_mount():
        raise ValueError("external destination is no longer mounted")
    device = mount.stat().st_dev
    if device == workspace.stat().st_dev or (expected_device is not None and device != expected_device):
        raise ValueError("external destination device changed or equals workspace device")
    if not destination_root.resolve().is_relative_to(mount.resolve()):
        raise ValueError("destination root escapes external mount")
    if destination_root.exists() and destination_root.stat().st_dev != device:
        raise ValueError("destination directory is not on the external device")
    return device


def _check_paths(name: str, workspace: Path, destination_root: Path, mount: Path,
                 *, require_mount: bool) -> tuple[Path, Path, Path, Path]:
    if name not in ALLOWLIST:
        raise ValueError(f"not in exact relocation allowlist: {name}")
    source = workspace / "build-opencv" / name
    dest = destination_root / name
    backup = source.with_name(name + ".relocation-backup")
    audit = destination_root / ".relocation-audits" / (name + ".json")
    if require_mount:
        verify_external_mount(mount, workspace, destination_root)
    if source.is_symlink() or not source.is_dir():
        raise ValueError("source missing or already relocated")
    if any(path.exists() or path.is_symlink() for path in (dest, backup, audit)):
        raise ValueError("destination, backup, or audit already exists; no overwrite")
    return source, dest, backup, audit


def relocate(name: str, *, workspace: Path = WORKSPACE,
             destination_root: Path = DEST_ROOT, mount: Path = MOUNT,
             min_free: int = MIN_EXTERNAL_FREE, require_mount: bool = True) -> dict:
    source, dest, backup, audit = _check_paths(
        name, workspace, destination_root, mount, require_mount=require_mount
    )
    external_device = mount.stat().st_dev
    before = inventory(source)
    total = sum(item["bytes"] for item in before["files"].values())
    if shutil.disk_usage(mount).free < min_free + total + COPY_BUFFER:
        raise ValueError("external disk lacks source size + 10 GiB margin + copy buffer")
    destination_root.mkdir(parents=True, exist_ok=True)
    # Symlinks are copied as symlinks rather than followed; the copied inventory
    # then rejects them. A changed source is also caught by the second hash pass.
    shutil.copytree(source, dest, copy_function=shutil.copy2, symlinks=True)
    copied = inventory(dest)
    source_after_copy = inventory(source)
    if copied != before or source_after_copy != before:
        raise ValueError("source/copy inventory changed; original and copy retained")
    if shutil.disk_usage(mount).free < min_free:
        raise ValueError("external 10 GiB floor reached; original and copy retained")
    audit.parent.mkdir(parents=True, exist_ok=True)
    manifest = {
        "schema": "crispmesh_verified_relocation_v1",
        "name": name,
        "source": str(source),
        "destination": str(dest),
        "mount": str(mount),
        "inventory": before,
        "total_file_bytes": total,
        "source_verified_after_copy": True,
        "destination_verified_after_copy": True,
        "link_verified": False,
        "backup_removed": False,
    }
    # Before renaming the original, make a recoverable on-volume audit record.
    with audit.open("x") as stream:
        json.dump(manifest, stream, indent=2, sort_keys=True)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())
    source.rename(backup)
    try:
        source.symlink_to(dest, target_is_directory=True)
        if not source.is_symlink() or source.resolve(strict=True) != dest.resolve(strict=True):
            raise ValueError("relocation link does not resolve to verified copy")
        if inventory(dest) != before or inventory(backup) != before:
            raise ValueError("post-link inventory differs from pre-copy inventory")
    except Exception:
        if source.is_symlink():
            source.unlink()
        if not source.exists() and backup.exists():
            backup.rename(source)
        raise
    manifest["link_verified"] = True
    # Exact backup only, after verified destination and link. If deletion fails,
    # the verified destination remains available and the audit records the phase.
    try:
        sync_files(dest, list(before["files"]))
        if inventory(dest) != before or inventory(backup) != before:
            raise ValueError("post-flush source/copy inventory differs")
        if require_mount:
            verify_external_mount(mount, workspace, destination_root, expected_device=external_device)
        if dest.stat().st_dev != external_device or shutil.disk_usage(mount).free < min_free:
            raise ValueError("external destination changed or fell below disk floor")
    except Exception:
        # No deletion has begun. Restore the original path only if our exact
        # symlink and intact backup are still present; never replace an
        # unexpected user path.
        if (link_points_to_destination(source, dest) and
                backup.is_dir() and not backup.is_symlink()):
            source.unlink()
            backup.rename(source)
        raise
    shutil.rmtree(backup)
    manifest["backup_removed"] = True
    with audit.open("w") as stream:
        json.dump(manifest, stream, indent=2, sort_keys=True)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())
    return manifest


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", required=True, choices=sorted(ALLOWLIST))
    args = parser.parse_args()
    report = relocate(args.run)
    print(json.dumps({key: report[key] for key in ("name", "total_file_bytes", "link_verified", "backup_removed")}, indent=2))


if __name__ == "__main__":
    main()
