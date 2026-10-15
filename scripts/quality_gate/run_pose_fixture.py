#!/usr/bin/env python3
"""Build and run the independent rendered ArUco pose fixture."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[2]
BUILD = ROOT / "build-opencv"
RESERVE_BYTES = 10 * 1024**3


def digest(path):
    hash_ = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1 << 20), b""):
            hash_.update(block)
    return hash_.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    targets = [output, Path(str(output) + ".provenance.json")]
    targets += [Path(str(output) + f".view{i}.png") for i in range(8)]
    for target in targets:
        if target.exists() or target.is_symlink():
            parser.error(f"output already exists: {target}")
    if not output.parent.is_dir():
        parser.error(f"output parent does not exist: {output.parent}")
    if shutil.disk_usage(output.parent).free < RESERVE_BYTES + 64 * 1024**2:
        parser.error("10 GiB free-space reserve plus fixture headroom required")
    flags = (BUILD / "CMakeFiles/crispmesh_pose_integration.dir/flags.make").read_text()
    includes = shlex.split(re.search(r"^CXX_INCLUDES = (.*)$", flags, re.M).group(1))
    link = shlex.split((BUILD / "CMakeFiles/crispmesh_pose_integration.dir/link.txt").read_text())
    libraries = link[link.index("libcrispmesh_core.a") + 1:]
    source = ROOT / "scripts/quality_gate/pose_fixture.cpp"
    scratch = ROOT / ".local-tools" / "tmp"
    scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="pose-fixture-", dir=scratch) as temporary:
        binary = Path(temporary) / "pose-fixture"
        command = ["/usr/bin/c++", "-O2", "-std=c++20", *includes,
                   str(source), "-o", str(binary), *libraries]
        environment = dict(os.environ, TMPDIR=str(scratch))
        subprocess.run(command, cwd=BUILD, check=True, timeout=120,
                       env=environment)
        binary_hash = digest(binary)
        subprocess.run([str(binary), str(output)], check=True, timeout=120,
                       env=environment)
    report = json.loads(output.read_text())
    if len(report["views"]) != 8 or not all(
            view["default"]["success"] and view["subpix"]["success"]
            for view in report["views"]):
        raise RuntimeError("pose fixture did not fit all eight views")
    artifact_hashes = {path.name: digest(path) for path in targets[2:]}
    artifact_hashes[output.name] = digest(output)
    provenance = {
        "source_sha256": digest(source), "binary_sha256": binary_hash,
        "artifacts_sha256": artifact_hashes, "compile_command": command,
        "run_command": [str(binary), str(output)],
        "cwd": str(BUILD), "tmpdir": str(scratch),
    }
    with targets[1].open("x") as destination:
        destination.write(json.dumps(provenance, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
