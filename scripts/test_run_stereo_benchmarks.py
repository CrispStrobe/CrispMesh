"""Local matrix orchestration checks; they never download data or use /tmp."""

import json
import os
from pathlib import Path
import random
import shutil
import struct
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent.parent
RUNNER = ROOT / "scripts/run_stereo_benchmarks.py"
EVALUATOR = ROOT / "build-opencv/bin/crispmesh_stereo_eval"
CENSUS = ROOT / ".local-tools/oracles/census/census"
SCRATCH = Path(os.environ["TMPDIR"]) if os.environ.get("TMPDIR") else ROOT / ".local-tools/tmp"


def pgm(path: Path, rows: list[list[int]]) -> None:
    with path.open("wb") as output:
        output.write(f"P5\n{len(rows[0])} {len(rows)}\n255\n".encode())
        output.write(bytes(pixel for row in rows for pixel in row))


def pfm(path: Path, rows: list[list[float]]) -> None:
    with path.open("wb") as output:
        output.write(f"Pf\n{len(rows[0])} {len(rows)}\n-1.0\n".encode())
        for row in reversed(rows):
            output.write(struct.pack(f"<{len(row)}f", *row))


class MatrixRunnerTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        if not EVALUATOR.is_file() or not CENSUS.is_file():
            raise unittest.SkipTest("Build evaluator and census first")
        SCRATCH.mkdir(parents=True, exist_ok=True)
        if shutil.disk_usage(SCRATCH).free < 10 * 1024**3 + 128 * 1024**2:
            raise unittest.SkipTest("10 GiB storage reserve")

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="c3ds-matrix-", dir=SCRATCH)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        sources = self.root / "sources"
        sources.mkdir()
        prepared = self.root / "prepared"
        prepared.mkdir()
        rng = random.Random(30)
        left = [[rng.randrange(256) for _ in range(128)] for _ in range(48)]
        right = [[left[y][min(127, x + 7)] for x in range(128)] for y in range(48)]
        for name, rows in (("left.pgm", left), ("right.pgm", right)):
            pgm(sources / name, rows)
            shutil.copyfile(sources / name, prepared / name)
        truth = [[7.0] * 128 for _ in range(48)]
        pfm(sources / "truth.pfm", truth)
        shutil.copyfile(sources / "truth.pfm", prepared / "truth.pfm")
        (prepared / "inputs.json").write_text(json.dumps({
            "schema_version": 1, "left": "left.pgm", "right": "right.pgm", "truth": "truth.pfm",
            "source_left": str(sources / "left.pgm"), "source_right": str(sources / "right.pgm"),
            "source_gt": str(sources / "truth.pfm"), "width": 128, "height": 48,
            "ndisp": 16, "calibration": {"fx": 100, "baseline": 50, "doffs": 0, "ndisp": 16},
        }))
        self.prepared = prepared
        self.output = self.root / "results"
        self.fake_elas = self.root / "fake-elas"
        shutil.copy2(CENSUS, self.fake_elas)
        self.fake_elas.chmod(0o755)
        self.fake_script_number = 0

    def run_matrix(self, *extra: str) -> tuple[subprocess.CompletedProcess[str], dict | None]:
        process = subprocess.run([
            "python3", str(RUNNER), "--prepared", f"synthetic={self.prepared}",
            "--elas", str(self.fake_elas), "--census", str(CENSUS), "--evaluator", str(EVALUATOR),
            "--output-root", str(self.output), "--allow-unpinned", *extra,
        ], capture_output=True, text=True, timeout=30, cwd=ROOT, env={**os.environ, "TMPDIR": str(SCRATCH)})
        report = Path(process.stdout.strip()) if process.stdout.strip().endswith("benchmark.json") else None
        return process, json.loads(report.read_text()) if report and report.is_file() else None

    def fake_script(self, body: str) -> None:
        # On macOS, replacing a copied Mach-O executable with a script at the
        # same path can stall direct exec. Give each script a fresh identity.
        self.fake_script_number += 1
        self.fake_elas = self.root / f"fake-elas-script-{self.fake_script_number}"
        self.fake_elas.write_text("#!/usr/bin/env python3\n" + body + "\n")
        self.fake_elas.chmod(0o755)

    def test_fake_scripts_use_fresh_paths(self) -> None:
        original = self.fake_elas
        self.fake_script("import sys\nsys.exit(7)")
        first = self.fake_elas
        self.fake_script("import time\ntime.sleep(5)")
        self.assertNotEqual(first, original)
        self.assertNotEqual(self.fake_elas, first)
        self.assertTrue(original.is_file())
        self.assertIn("time.sleep(5)", self.fake_elas.read_text())

    def test_three_engines_share_prepared_inputs_and_search_domain(self) -> None:
        process, report = self.run_matrix()
        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertTrue(report["ok"])
        scene = report["scenes"]["synthetic"]
        self.assertEqual(scene["common_search_count"], 16)
        self.assertEqual(set(scene["engines"]), {"sgbm", "elas", "census"})
        for name, engine in scene["engines"].items():
            self.assertEqual(engine["status"], "ok", (name, engine))
            self.assertEqual(engine["search_domain"], [0, 15])
            self.assertEqual(engine["metrics"]["gt_valid"], 128 * 48)
            self.assertIn("binary_sha256", engine)
            if name != "sgbm":
                command = engine["steps"][0]["command"]
                self.assertNotIn("--gt", command)
                self.assertNotIn("--mask", command)
                self.assertEqual(command[command.index("--ndisp") + 1], "16")
        self.assertEqual(len(scene["inputs_sha256"]), 3)

    def test_nonzero_exit_and_timeout_are_reported(self) -> None:
        self.fake_script("import sys\nsys.exit(7)")
        process, report = self.run_matrix()
        self.assertEqual(process.returncode, 1, process.stderr)
        self.assertFalse(report["ok"])
        self.assertEqual(report["scenes"]["synthetic"]["engines"]["elas"]["steps"][0]["exit_code"], 7)
        self.assertEqual(report["scenes"]["synthetic"]["engines"]["census"]["status"], "ok")
        self.fake_script("import time\ntime.sleep(5)")
        process, report = self.run_matrix("--timeout-seconds", "1")
        self.assertEqual(process.returncode, 1, process.stderr)
        self.assertEqual(report["scenes"]["synthetic"]["engines"]["elas"]["steps"][0]["status"], "timeout")

    def test_input_mutation_is_reported(self) -> None:
        self.fake_script("import sys\nfrom pathlib import Path\np=Path(sys.argv[sys.argv.index('--left')+1])\np.write_bytes(p.read_bytes()[:-1]+b'X')")
        process, report = self.run_matrix()
        self.assertEqual(process.returncode, 1, process.stderr)
        self.assertFalse(report["ok"])
        self.assertIn("changed during benchmark", " ".join(report["failures"]))

    def test_duplicate_names_rejected_before_run(self) -> None:
        process, report = self.run_matrix("--prepared", f"synthetic={self.prepared}")
        self.assertNotEqual(process.returncode, 0)
        self.assertIsNone(report)
        self.assertIn("unique", process.stderr)


if __name__ == "__main__":
    unittest.main()
