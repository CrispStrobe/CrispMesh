"""Evaluation-only, configure-only OpenMVG v2.1 LiGT-off fork supervisor.

Default invocation is read-only. --configure-fork is a separately approved,
one-shot operation; it never compiles, stages photos, or runs SfM.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import time

from scripts.classical_backend import openmvg_m1_supervisor as base


ROOT = Path("/Volumes/backups/code/crispmesh-data/openmvg-v21-ligt-off-oracle-001")
OLD_SOURCE = base.DEFAULT_ROOT / "source"
INTERNAL = base.DEFAULT_INTERNAL
PATCH = INTERNAL / "scripts/classical_backend/openmvg_v21_ligt_off.patch"
FIXTURE = INTERNAL / "scripts/classical_backend/fixtures/openmvg_v21_multiview_CMakeLists.txt"
PATCH_SHA = "472b4a925cc5e92fc41c733777d55b4cee2d8944764441537f69689968510f2c"
SOURCE_CMAKE_SHA = "139ed9e2793e907c3fa74ea805c5b418336f51812ab7c56036aeb14e866233a3"
TREE_CAP = 1024 ** 3
FLOOR = 11 * 1024 ** 3
RSS_CAP_KIB = 4 * 1024 ** 2
LOG_CAP = 16 * 1024 ** 2
COPY_TIMEOUT = 180
PATCHED_REL = Path("src/openMVG/multiview/CMakeLists.txt")
FORBIDDEN_GRAPH = re.compile(r"(?:LiGT[/\\][^\s:]*\.(?:cpp|hpp|o|obj)|USE_PATENTED_LIGT)", re.I)
GIT_CONTEXT_KEYS = ("GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR",
                    "GIT_PREFIX", "GIT_OBJECT_DIRECTORY", "GIT_ALTERNATE_OBJECT_DIRECTORIES")


def reject_git_context() -> None:
    if any(os.environ.get(key) for key in GIT_CONTEXT_KEYS):
        raise RuntimeError("inherited Git worktree/index context is unsafe for sealed fork")


def seal_patch_and_source() -> dict:
    if base.sha256(PATCH) != PATCH_SHA or base.sha256(FIXTURE) != SOURCE_CMAKE_SHA:
        raise RuntimeError("pinned patch/fixture seal mismatch")
    inventory = base.source_inventory(OLD_SOURCE)
    if base.sha256(OLD_SOURCE / PATCHED_REL) != SOURCE_CMAKE_SHA:
        raise RuntimeError("original CMake source seal mismatch")
    base.reject_implicit_fetches(OLD_SOURCE)
    eigen = base.verify_local_eigen()
    return {"source": inventory, "patch_sha256": PATCH_SHA,
            "source_cmake_sha256": SOURCE_CMAKE_SHA, "eigen_sha256": eigen}


def capacity(root: Path = ROOT, prospective: bool = False) -> dict:
    base.approved_root(root)
    used = base.tree_bytes(root)
    external = base.free_bytes(root)
    internal = base.free_bytes(INTERNAL)
    required = FLOOR + (max(0, TREE_CAP - used) if prospective else 0)
    if used > TREE_CAP or external < required or internal < FLOOR:
        raise RuntimeError(f"fork capacity: used={used}, external={external}, "
                           f"need={required}, internal={internal}, floor={FLOOR}")
    return {"fork_bytes": used, "external_free": external,
            "external_required": required, "internal_free": internal}


def configure_command(source: Path, build: Path) -> list[str]:
    return base.eigen_configure_command(source, build) + ["-DCMAKE_EXPORT_COMPILE_COMMANDS=ON"]


def preflight(root: Path = ROOT) -> dict:
    if root.exists():
        raise RuntimeError(f"fresh fork root already exists: {root}")
    reject_git_context()
    seals = seal_patch_and_source()
    space = capacity(root, prospective=True)
    if not shutil.which("cmake") or not shutil.which("ninja"):
        raise RuntimeError("existing CMake and Ninja required; no installs")
    if base.tree_bytes(OLD_SOURCE) > TREE_CAP:
        raise RuntimeError("sealed source copy cannot fit fork tree cap")
    return {"root": str(root), "old_source": str(OLD_SOURCE),
            "source_bytes": base.tree_bytes(OLD_SOURCE), "seals": seals,
            "capacity": space, "configure": configure_command(root / "source", root / "build"),
            "targets": base.TARGETS, "operation": "configure only; no compile/photos"}


def save_manifest(root: Path, manifest: dict) -> None:
    pending = root / "tmp/manifest.pending.json"
    pending.write_text(json.dumps(manifest, sort_keys=True, indent=2) + "\n")
    pending.replace(root / "fork-manifest.json")


def copy_sealed_source(destination: Path, root: Path) -> int:
    """Local byte copy only; bounded incremental counter, retaining partial tree on stop."""
    started = time.monotonic()
    used = base.tree_bytes(root)
    count = 0
    destination.mkdir()
    for base_dir, dirs, files in os.walk(OLD_SOURCE, followlinks=False):
        relative = Path(base_dir).relative_to(OLD_SOURCE)
        target_dir = destination / relative
        for name in dirs:
            item = Path(base_dir) / name
            if item.is_symlink():
                raise RuntimeError(f"symlink in sealed source: {item}")
            (target_dir / name).mkdir(exist_ok=False)
        for name in files:
            if time.monotonic() - started > COPY_TIMEOUT:
                raise RuntimeError("sealed source copy exceeded 180-second wall-time cap")
            item = Path(base_dir) / name
            if item.is_symlink() or not item.is_file():
                raise RuntimeError(f"unsupported source item: {item}")
            size = item.stat().st_size
            if used + size > TREE_CAP:
                raise RuntimeError("source copy would exceed 1 GiB tree cap")
            if base.free_bytes(root) < FLOOR + size or base.free_bytes(INTERNAL) < FLOOR:
                raise RuntimeError("disk floor reached during source copy")
            shutil.copy2(item, target_dir / name)
            used += size
            count += 1
            if count % 32 == 0:
                capacity(root)
    capacity(root)
    if time.monotonic() - started > COPY_TIMEOUT:
        raise RuntimeError("sealed source copy exceeded 180-second wall-time cap")
    return count


def fork_source_seal(source: Path, expected_inventory: dict) -> None:
    if base.source_inventory(source) != expected_inventory:
        raise RuntimeError("local fork copy differs from sealed original source")
    if base.sha256(source / PATCHED_REL) != SOURCE_CMAKE_SHA:
        raise RuntimeError("fork CMake source changed before patch")


def expected_patched_text(original: str) -> str:
    substitutions = (
        ("file(GLOB_RECURSE REMOVEFILELIGT LiGT_*.cpp)", "file(GLOB REMOVEFILELIGT ./LiGT/*.cpp)"),
        ("file(GLOB_RECURSE REMOVEFILELIGT_HEADER LiGT_*.hpp)", "file(GLOB REMOVEFILELIGT_HEADER ./LiGT/*.hpp)"),
        ("list(REMOVE_ITEM multiview_files_cpp ${REMOVEFILELIGT_HEADER})",
         "list(REMOVE_ITEM multiview_files_header ${REMOVEFILELIGT_HEADER})"),
    )
    for old, new in substitutions:
        if original.count(old) != 1:
            raise RuntimeError(f"pinned CMake patch context changed: {old}")
        original = original.replace(old, new)
    return original


def verify_patched_fork(source: Path, original_inventory: dict) -> dict:
    target = source / PATCHED_REL
    expected = expected_patched_text(FIXTURE.read_text())
    if target.read_text() != expected:
        raise RuntimeError("fork patch is not exactly the reviewed LiGT-off change")
    if base.git_output(source, "rev-parse", "HEAD") != base.SOURCE_REV:
        raise RuntimeError("fork revision changed")
    status = base.git_output(source, "status", "--porcelain")
    if status.strip() != "M src/openMVG/multiview/CMakeLists.txt":
        raise RuntimeError(f"fork has unexpected edits: {status}")
    for rel, (revision, _) in base.SUBMODULES.items():
        if base.git_output(source / rel, "rev-parse", "HEAD") != revision:
            raise RuntimeError(f"fork submodule changed: {rel}")
    if original_inventory["files_sha256"] != {
            rel: base.sha256(source / rel) for rel in original_inventory["files_sha256"]}:
        raise RuntimeError("fork license/source inventory changed outside patch")
    return {"patched_cmake_sha256": base.sha256(target), "git_status": status}


def graph_gate(build: Path, configure_log: Path, ninja_commands: str) -> dict:
    """Reject LiGT source/header/object inputs or patented compile definition before build."""
    base.verify_pinned_eigen_configuration(build, configure_log)
    cache = base.cache_entries((build / "CMakeCache.txt").read_text())
    if cache.get("CMAKE_EXPORT_COMPILE_COMMANDS") != "ON":
        raise RuntimeError("compile command export was not enabled")
    commands = json.loads((build / "compile_commands.json").read_text())
    if not commands or not isinstance(commands, list):
        raise RuntimeError("missing generated compile commands")
    for entry in commands:
        line = " ".join(str(entry.get(key, "")) for key in ("file", "command", "output"))
        if FORBIDDEN_GRAPH.search(line):
            raise RuntimeError(f"LiGT/patented compile input in compile_commands: {line[:200]}")
    ninja = (build / "build.ninja").read_text(errors="replace")
    for label, value in (("Ninja graph", ninja), ("four-target closure", ninja_commands)):
        hit = FORBIDDEN_GRAPH.search(value)
        if hit:
            raise RuntimeError(f"LiGT/patented input in {label}: {hit.group()}")
    for target in base.TARGETS:
        exact = re.compile(r"(?<![A-Za-z0-9_])" + re.escape(target) + r"(?![A-Za-z0-9_])")
        if not exact.search(ninja) or not exact.search(ninja_commands):
            raise RuntimeError(f"required CLI target missing from generated graph: {target}")
    return {"compile_units": len(commands), "ninja_sha256": hashlib.sha256(ninja.encode()).hexdigest(),
            "compile_commands_sha256": base.sha256(build / "compile_commands.json"),
            "target_commands_sha256": hashlib.sha256(ninja_commands.encode()).hexdigest(),
            "forbidden_inputs": 0}


def run_logged(command: list[str], root: Path, log_path: Path, manifest: dict,
               timeout: int) -> None:
    record = {"command": command, "log": str(log_path), "started_utc":
              datetime.now(timezone.utc).isoformat(), "status": "running"}
    manifest["stages"].append(record)
    save_manifest(root, manifest)
    started = time.monotonic()
    proc = None
    try:
        with log_path.open("xb") as log:
            env = os.environ.copy()
            for key in ("TMPDIR", "TMP", "TEMP", "XDG_CACHE_HOME"):
                env[key] = str(root / "tmp")
            env["HOMEBREW_NO_AUTO_UPDATE"] = "1"
            env["GIT_CONFIG_NOSYSTEM"] = "1"
            env["GIT_CONFIG_GLOBAL"] = "/dev/null"
            for key in GIT_CONTEXT_KEYS:
                env.pop(key, None)
            proc = subprocess.Popen(command, env=env, stdout=log, stderr=subprocess.STDOUT,
                                    start_new_session=True)
            while proc.poll() is None:
                if time.monotonic() - started > timeout:
                    raise RuntimeError("configure/graph timeout")
                if log_path.stat().st_size > LOG_CAP:
                    raise RuntimeError("16 MiB stage log cap exceeded")
                capacity(root)
                table = subprocess.check_output(["/bin/ps", "-axo", "pid=,ppid=,rss="], text=True)
                if base.process_rss_kib(proc.pid, table) > RSS_CAP_KIB:
                    raise RuntimeError("4 GiB process-tree RSS cap exceeded")
                time.sleep(0.5)
            if proc.returncode or log_path.stat().st_size > LOG_CAP:
                raise RuntimeError(f"stage returned {proc.returncode} or exceeded log cap")
        capacity(root)
        record["status"] = "completed"
    except BaseException as exc:
        record["status"] = "stopped"
        record["reason"] = str(exc)
        if proc is not None and proc.poll() is None:
            os.killpg(proc.pid, signal.SIGTERM)
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(proc.pid, signal.SIGKILL)
                proc.wait()
        raise
    finally:
        record["elapsed_seconds"] = round(time.monotonic() - started, 3)
        record["returncode"] = proc.returncode if proc else None
        if log_path.exists() and log_path.stat().st_size > LOG_CAP:
            with log_path.open("r+b") as log:
                log.truncate(LOG_CAP)
            record["log_truncated_at_cap"] = True
        record["log_sha256"] = base.sha256(log_path) if log_path.exists() else None
        save_manifest(root, manifest)


def configure_fork(root: Path = ROOT) -> dict:
    checked = preflight(root)
    root.mkdir()
    for part in ("logs", "tmp"):
        (root / part).mkdir()
    manifest = {"schema": "openmvg_ligt_fork_configure_only_v1", "input": checked,
                "tree_cap_bytes": TREE_CAP, "floor_bytes": FLOOR,
                "rss_cap_kib": RSS_CAP_KIB, "log_cap_bytes": LOG_CAP,
                "stages": []}
    save_manifest(root, manifest)
    try:
        copy_started = time.monotonic()
        copy_record = {"operation": "local sealed source copy", "status": "running",
                       "started_utc": datetime.now(timezone.utc).isoformat(),
                       "wall_cap_seconds": COPY_TIMEOUT}
        manifest["stages"].append(copy_record)
        save_manifest(root, manifest)
        try:
            copy_record["files"] = copy_sealed_source(root / "source", root)
            fork_source_seal(root / "source", checked["seals"]["source"])
            copy_record["status"] = "completed"
        except BaseException as exc:
            copy_record["status"] = "stopped"
            copy_record["reason"] = str(exc)
            raise
        finally:
            copy_record["elapsed_seconds"] = round(time.monotonic() - copy_started, 3)
            save_manifest(root, manifest)
        # This local patch step has no network access and changes only the new fork.
        if base.sha256(PATCH) != PATCH_SHA:
            raise RuntimeError("patch changed after preflight")
        run_logged(["/usr/bin/git", "-C", str(root / "source"), "apply", "--check", str(PATCH)],
                   root, root / "logs/01-patch-check.log", manifest, 60)
        run_logged(["/usr/bin/git", "-C", str(root / "source"), "apply", str(PATCH)],
                   root, root / "logs/02-patch.log", manifest, 60)
        manifest["patched_fork"] = verify_patched_fork(root / "source", checked["seals"]["source"])
        save_manifest(root, manifest)
        base.reject_implicit_fetches(root / "source")
        run_logged(configure_command(root / "source", root / "build"), root,
                   root / "logs/03-configure.log", manifest, 600)
        commands = ["ninja", "-C", str(root / "build"), "-t", "commands", *base.TARGETS]
        run_logged(commands, root, root / "logs/04-target-commands.log", manifest, 120)
        target_commands = (root / "logs/04-target-commands.log").read_text(errors="replace")
        manifest["graph_gate"] = graph_gate(root / "build", root / "logs/03-configure.log",
                                             target_commands)
        manifest["status"] = "configured_graph_clean_no_compile"
        save_manifest(root, manifest)
        return manifest
    except BaseException as exc:
        manifest["status"] = "stopped"
        manifest["stop_reason"] = str(exc)
        save_manifest(root, manifest)
        raise


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--configure-fork", action="store_true",
                        help="approved fresh-copy/patch/configure/graph check; no compile/photos")
    args = parser.parse_args()
    result = configure_fork() if args.configure_fork else preflight()
    print(json.dumps(result, sort_keys=True, indent=2))


if __name__ == "__main__":
    main()
