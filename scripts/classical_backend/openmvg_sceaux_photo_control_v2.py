"""Fresh one-shot Sceaux control after the preserved -001 feature-CLI failure.

Default invocation is a read-only preflight. No prior output is reused.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import re
import shutil
import subprocess
import time

from scripts.classical_backend import openmvg_photo_control as base

FAILED = Path("/Volumes/backups/code/crispmesh-data/openmvg-sceaux-photo-sfm-001")
OUTPUT = Path("/Volumes/backups/code/crispmesh-data/openmvg-sceaux-photo-sfm-002")
FAILED_SEALS = {
    "receipt.json": "555f394b32f565bf0232e16899193c638871d4f614d81b10969a3168ac6f230f",
    "logs/01-listing.log": "f4c22fa1badd2dc39a5086c1e4495f9b6736a3e2d55afc0205eaf28e5bce7687",
    "logs/02-features.log": "d8725e6169614bd05cf5f338930cc10abfa5aab0dcbcc42c3089e53ebac1edf3",
    "matches/sfm_data.json": "def7ca0d3cec6996625e20a358aee66a6589d6e759c29f125a5cd13ae63dc110",
}


def verify_failed_first(failed: Path = FAILED) -> dict:
    if failed.is_symlink() or not failed.is_dir():
        raise RuntimeError("preserved -001 output missing")
    for name, expected in FAILED_SEALS.items():
        item = failed / name
        if item.is_symlink() or not item.is_file() or base.sha(item) != expected:
            raise RuntimeError(f"preserved -001 seal changed: {name}")
    receipt = json.loads((failed / "receipt.json").read_text())
    stages = receipt.get("stages", [])
    if (receipt.get("status") != "stopped" or len(stages) != 2 or
            [item.get("status") for item in stages] != ["completed", "stopped"] or
            [item.get("name") for item in stages] != ["listing", "features"] or
            stages[1].get("command", [])[-2:] != ["-n", "2"] or
            receipt.get("output_inventory") != base.output_inventory(failed)):
        raise RuntimeError("preserved -001 failed-stage contract changed")
    return {"root": str(failed), "sha256": FAILED_SEALS,
            "status": "stopped_at_feature_parser"}


def commands(output: Path, build: Path = base.BUILD) -> list[tuple[str, list[str], str]]:
    stages = base.commands(output, build)
    name, feature, artifact = stages[1]
    if name != "features" or feature[-2:] != ["-n", "2"]:
        raise RuntimeError("inherited v1 command contract changed")
    stages[1] = name, feature[:-2], artifact
    return stages


def parse_usage_options(usage: str) -> set[str]:
    return set(re.findall(r"\[-([A-Za-z])\|--[A-Za-z_]+", usage))


def audit_cli_usage(stages: list[tuple[str, list[str], str]]) -> dict:
    """Run each sealed binary with no arguments; it prints usage and reads no photos."""
    observed = {}
    for name, argv, _ in stages:
        result = subprocess.run([argv[0]], capture_output=True, timeout=10)
        usage = (result.stdout + result.stderr).decode(errors="replace")
        if result.returncode != 1 or len(usage.encode()) > 32768 or "Usage:" not in usage:
            raise RuntimeError(f"unexpected no-argument usage behavior: {name}")
        options = parse_usage_options(usage)
        planned = [argv[i] for i in range(1, len(argv), 2)]
        if any(len(option) != 2 or not option.startswith("-") or option[1] not in options
               for option in planned):
            raise RuntimeError(f"planned CLI option absent from compiled {name} usage")
        if name == "features" and "n" in options:
            raise RuntimeError("feature CLI unexpectedly exposes OpenMP-only -n")
        observed[name] = {"options": sorted(options), "planned": planned,
                          "binary_sha256": base.sha(Path(argv[0]))}
    return observed


def preflight(output: Path = OUTPUT, failed: Path = FAILED) -> dict:
    checked = base.preflight(output)
    checked["schema"] = "openmvg_sceaux_photo_control_v2"
    checked["failed_first"] = verify_failed_first(failed)
    checked["compiled_cli_usage"] = audit_cli_usage(commands(output))
    return checked


def run(output: Path = OUTPUT) -> dict:
    checked = preflight(output)
    output.mkdir()
    for name in ("images", "matches", "sparse", "logs", "tmp"):
        (output / name).mkdir()
    receipt = {**checked, "status": "running", "started_utc": datetime.now(timezone.utc).isoformat(),
               "stages": []}
    base.save(output, receipt)
    try:
        for name in base.NAMES:
            source, target = base.SAMPLE / "images" / name, output / "images" / name
            shutil.copyfile(source, target)
            if base.sha(target) != checked["photos"]["photos"][name]:
                raise RuntimeError(f"staged photo differs: {name}")
            base.capacity(output)
        if base.verify_photos() != checked["photos"]:
            raise RuntimeError("sealed photos changed during staging")
        receipt["staged_photo_sha256"] = {name: base.sha(output / "images" / name)
                                           for name in base.NAMES}
        base.save(output, receipt)
        total_start = time.monotonic()
        for index, (name, command, expected) in enumerate(commands(output)):
            record = {"name": name, "status": "running", "command": command}
            receipt["stages"].append(record)
            base.save(output, receipt)
            try:
                record.update(base.run_stage(command, output / "logs" / f"{index + 1:02}-{name}.log",
                                             output, base.STAGE_SECONDS[index], total_start))
                artifact = Path(expected)
                if artifact.is_symlink() or not artifact.is_file() or artifact.stat().st_size == 0:
                    raise RuntimeError(f"required stage artifact missing: {artifact}")
                record["artifact_sha256"] = base.sha(artifact)
                record["status"] = "completed"
            except BaseException as exc:
                record["status"] = "stopped"
                record["reason"] = str(exc)
                raise
            finally:
                record["output_bytes"] = base.base.tree_bytes(output)
                base.save(output, receipt)
        receipt["sfm_report"] = base.report_counts(output / "sparse/SfMReconstruction_Report.html")
        receipt["status"] = ("completed_pending_geometry_review"
                             if receipt["sfm_report"]["poses"] == len(base.NAMES)
                             and receipt["sfm_report"]["tracks"] > 0
                             and receipt["sfm_report"]["residuals"] > 0
                             else "failed_registration_or_sparse_gate")
    except BaseException as exc:
        receipt["status"] = "stopped"
        receipt["reason"] = str(exc)
        raise
    finally:
        receipt["finished_utc"] = datetime.now(timezone.utc).isoformat()
        receipt["output_inventory"] = base.output_inventory(output)
        base.save(output, receipt)
    return receipt


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run-sceaux", action="store_true", help="one-shot live photo run; requires review")
    args = parser.parse_args()
    print(json.dumps(run() if args.run_sceaux else preflight(), indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
