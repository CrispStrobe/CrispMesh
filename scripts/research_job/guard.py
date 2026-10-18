"""Run one local research command with disk, log, and time bounds.

Supported on macOS and Linux. This is a resource guard, not a security sandbox:
callers must pass trusted commands and direct all generated files to output_dir.
"""

from __future__ import annotations

from datetime import datetime, timezone
import json
import math
import os
from pathlib import Path
import platform
import shutil
import signal
import stat
import subprocess
import time


POLL_SECONDS = 0.1
TEMP_ROOT = Path(__file__).resolve().parents[2] / ".local-tools" / "tmp"


def _no_symlink_components(path: Path) -> None:
    """Reject links, including a dangling link at the final component."""
    for component in (path, *path.parents):
        if component.is_symlink():
            raise ValueError(f"symlink path is not allowed: {component}")


def _tree_bytes(path: Path) -> int:
    total = 0
    pending = [path]
    while pending:
        directory = pending.pop()
        try:
            with os.scandir(directory) as entries:
                for entry in entries:
                    try:
                        info = entry.stat(follow_symlinks=False)
                    except FileNotFoundError:
                        # The child may release an intermediate after enumeration, before stat.
                        continue
                    if stat.S_ISLNK(info.st_mode):
                        raise ValueError(f"symlink appeared in run output: {entry.path}")
                    if stat.S_ISDIR(info.st_mode):
                        pending.append(Path(entry.path))
                    elif stat.S_ISREG(info.st_mode):
                        total += info.st_size
                    else:
                        raise ValueError(f"special file appeared in run output: {entry.path}")
        except FileNotFoundError:
            # Likewise, a directory queued above may have been removed in the meantime.
            continue
    return total


def _kill_group(proc: subprocess.Popen[bytes]) -> None:
    try:
        os.killpg(proc.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    except PermissionError:
        # macOS may deny killpg once the leader has already exited.
        if proc.poll() is None:
            proc.kill()
    proc.wait()


def run_child(
    command: list[str],
    cwd: Path | str,
    output_dir: Path | str,
    timeout_seconds: float = 300,
    max_output_bytes: int = 1 << 30,
    reserve_bytes: int = 10 << 30,
    max_log_bytes: int = 10 << 20,
) -> dict:
    """Run a trusted argv in its own process group and return the saved status.

    A fresh output directory is mandatory. Status and combined log remain there
    after child failures. Preflight failures raise ValueError without creating it.
    """
    if platform.system() not in {"Darwin", "Linux"}:
        raise ValueError("research job guard supports macOS and Linux only")
    if not isinstance(command, list) or not command or not all(
        isinstance(arg, str) and arg and "\x00" not in arg for arg in command
    ):
        raise ValueError("command must be a nonempty list of nonempty strings")
    if (isinstance(timeout_seconds, bool) or
            not isinstance(timeout_seconds, (int, float)) or
            not math.isfinite(timeout_seconds) or timeout_seconds <= 0):
        raise ValueError("timeout_seconds must be finite and positive")
    for name, value in (("max_output_bytes", max_output_bytes),
                        ("max_log_bytes", max_log_bytes)):
        if isinstance(value, bool) or not isinstance(value, int) or value <= 0:
            raise ValueError(f"{name} must be a positive integer")
    if isinstance(reserve_bytes, bool) or not isinstance(reserve_bytes, int) or reserve_bytes < 0:
        raise ValueError("reserve_bytes must be a nonnegative integer")
    work = Path(cwd).absolute()
    output = Path(output_dir).absolute()
    _no_symlink_components(work)
    _no_symlink_components(output)
    _no_symlink_components(TEMP_ROOT)
    if not work.is_dir() or not output.parent.is_dir():
        raise ValueError("cwd and output parent must be existing directories")
    if output.exists() or output.is_symlink():
        raise ValueError("output_dir must be fresh")
    if not TEMP_ROOT.parent.is_dir():
        raise ValueError(".local-tools must exist for task-local temporary files")
    if shutil.disk_usage(output.parent).free < reserve_bytes + max_output_bytes:
        raise ValueError("output volume lacks output cap plus free-space reserve")
    if shutil.disk_usage(TEMP_ROOT.parent).free < reserve_bytes:
        raise ValueError("temporary volume lacks free-space reserve")

    output.mkdir()  # Fails if another process has claimed this run directory.
    temporary = TEMP_ROOT / f"research-{os.getpid()}-{time.monotonic_ns()}"
    log_path = output / "child.log"
    started = time.monotonic()
    reason = "succeeded"
    returncode = None
    proc = None
    error_type = None
    interrupted = None
    try:
        TEMP_ROOT.mkdir(exist_ok=True)
        temporary.mkdir()
        env = os.environ.copy()
        env["TMPDIR"] = str(temporary)
        env["TMP"] = str(temporary)
        env["TEMP"] = str(temporary)
        with log_path.open("xb") as log:
            proc = subprocess.Popen(command, cwd=work, env=env, stdin=subprocess.DEVNULL,
                                    stdout=log, stderr=subprocess.STDOUT,
                                    start_new_session=True)
            while True:
                if time.monotonic() - started >= timeout_seconds:
                    reason = "timeout"
                elif log_path.stat().st_size > max_log_bytes:
                    reason = "log_cap_exceeded"
                elif _tree_bytes(output) + _tree_bytes(temporary) > max_output_bytes:
                    reason = "output_cap_exceeded"
                elif (shutil.disk_usage(output).free < reserve_bytes or
                      shutil.disk_usage(temporary).free < reserve_bytes):
                    reason = "free_space_reserve_breached"
                if reason != "succeeded":
                    _kill_group(proc)
                    break
                if proc.poll() is not None:
                    # The parent may exit after leaving a writer in its group.
                    _kill_group(proc)
                    break
                time.sleep(POLL_SECONDS)
            returncode = proc.wait()
            if reason == "succeeded" and returncode != 0:
                reason = "nonzero_exit"
    except BaseException as exc:
        reason = "guard_error"
        if proc is not None:
            _kill_group(proc)
            returncode = proc.returncode
        error_type = type(exc).__name__
        if not isinstance(exc, Exception):
            interrupted = exc
    finally:
        # The temporary directory is exclusively created by this invocation.
        # Reject unexpected links before cleanup to avoid traversing child links.
        cleanup_error = None
        try:
            if temporary.exists() or temporary.is_symlink():
                _tree_bytes(temporary)
                shutil.rmtree(temporary)
        except (OSError, ValueError) as exc:
            cleanup_error = type(exc).__name__
        status = {"status": reason, "returncode": returncode,
                  "duration_seconds": round(time.monotonic() - started, 3),
                  "finished_utc": datetime.now(timezone.utc).isoformat(),
                  "command": command}
        if error_type is not None:
            status["error_type"] = error_type
        if cleanup_error is not None:
            status["temporary_cleanup_error"] = cleanup_error
            status["temporary_directory"] = str(temporary)
        with (output / "status.json").open("x", encoding="utf-8") as report:
            json.dump(status, report, sort_keys=True, indent=2)
            report.write("\n")
    if interrupted is not None:
        raise interrupted
    return status
