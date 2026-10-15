#!/usr/bin/env python3
"""Build the project-authored Census comparator outside shipped app targets."""

import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess


ROOT = Path(__file__).resolve().parents[3]
SOURCE = Path(__file__).with_name("census.cpp")
OUTPUT = ROOT / ".local-tools/oracles/census/census"
SCRATCH = ROOT / ".local-tools/tmp"
RESERVE_BYTES = 10 * 1024**3


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def main() -> None:
    free = shutil.disk_usage(ROOT).free
    if free < RESERVE_BYTES + 128 * 1024**2:
        raise SystemExit(f"Build would breach 10 GiB free-space reserve ({free} bytes free)")
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    SCRATCH.mkdir(parents=True, exist_ok=True)
    compiler = shutil.which("clang++") or shutil.which("c++")
    if not compiler:
        raise SystemExit("No C++20 compiler found")
    command = [compiler, "-O3", "-std=c++20", "-Wall", "-Wextra", "-pedantic", str(SOURCE), "-o", str(OUTPUT)]
    environment = dict(os.environ, TMPDIR=str(SCRATCH))
    subprocess.run(command, check=True, env=environment)
    provenance = {
        "name": "crispmesh_census_reference",
        "role": "project-authored reference comparator; not an external oracle or production stage",
        "method": "5x5 Census/Hamming plus intensity cost, 3x3 box aggregation, integer WTA",
        "invalid_disparity": "+Infinity at unsupported borders; zero is a valid candidate",
        "source": str(SOURCE.relative_to(ROOT)),
        "source_sha256": digest(SOURCE),
        "binary_sha256": digest(OUTPUT),
        "compiler": subprocess.check_output([compiler, "--version"], text=True).splitlines()[0],
        "compile_command": command,
        "platform": platform.platform(),
    }
    (OUTPUT.parent / "provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")
    print(OUTPUT)


if __name__ == "__main__":
    main()
