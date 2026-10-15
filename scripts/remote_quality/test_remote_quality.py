"""Local safety checks; no remote calls or large data."""

import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
import zipfile

from package import make_package, validate_file
from prepare_kaggle import prepare
import run_cpu
from run_cpu import preflight


class RemoteQualityTest(unittest.TestCase):
    def test_package_allowlist_and_secret_rejection(self):
        with tempfile.TemporaryDirectory(dir=Path(tempfile.gettempdir()).resolve()) as tmp:
            base = Path(tmp)
            root = base / "repo"
            root.mkdir()
            (root / "safe.txt").write_text("safe\n")
            (root / ".env").write_text("not a credential\n")
            (root / "secret.txt").write_text("KGAT_" + "A" * 32)
            (root / "payload.txt").write_text("KGAT_" + "B" * 32)
            (root / "link.txt").symlink_to(root / "safe.txt")
            with self.assertRaises(ValueError):
                validate_file(root, "../safe.txt")
            with self.assertRaises(ValueError):
                validate_file(root, "a//b")
            for name in (".env", "link.txt", "build-opencv/a"):
                with self.assertRaises(ValueError):
                    validate_file(root, name)
            with self.assertRaises(ValueError):
                make_package(root, ["secret.txt"], base / "secret.zip", cap=1024)
            with self.assertRaises(ValueError):
                make_package(root, ["payload.txt"], base / "payload.zip", cap=1024)
            with self.assertRaises(ValueError):
                make_package(root, ["safe.txt"], base / "tiny.zip", cap=1)
            result = make_package(root, ["safe.txt"], base / "safe.zip", cap=1024)
            self.assertEqual(result["status"], "ready")
            with self.assertRaises(ValueError):
                make_package(root, ["safe.txt"], base / "safe.zip", cap=1024)

    def test_platform_guard(self):
        with self.assertRaisesRegex(ValueError, "Linux VPS only"):
            preflight({}, system="Darwin")

    def test_unpacked_size_checked_before_read(self):
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            archive = base / "input.zip"
            manifest = {"schema": 1, "files": {"x.txt": {"bytes": 9, "sha256": "0" * 64}}}
            with zipfile.ZipFile(archive, "w") as z:
                z.writestr("remote-quality-manifest.json", json.dumps(manifest))
                z.writestr("x.txt", "123456789")
            work = base / "work"
            work.mkdir()
            with patch.object(run_cpu, "MAX_INPUT", 8), self.assertRaisesRegex(ValueError, "size mismatch"):
                run_cpu.unpack_checked(archive, work)
            self.assertFalse((work / "x.txt").exists())

    def test_result_required_and_aggregate_cap_stops_process(self):
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            archive = base / "input.zip"
            manifest = {"schema": 1, "files": {}}
            with zipfile.ZipFile(archive, "w") as z:
                z.writestr("remote-quality-manifest.json", json.dumps(manifest))
            ready = {"status": "ready", "threads": 1, "timeout_seconds": 5, "scratch_cap_bytes": 100_000}
            with patch.object(run_cpu, "FAST", base), patch.object(run_cpu, "STORAGE", base), \
                    patch.object(run_cpu, "MIN_FREE", 0):
                config = {"input_archive": str(archive), "work_dir": str(base / "work1"),
                          "output_dir": str(base / "out1"), "result_file": "metrics.json",
                          "command": [sys.executable, "-c", "pass"]}
                result = run_cpu.run(config, ready)
                self.assertEqual(result["status"], "missing_or_large_result")
                config["work_dir"] = str(base / "work2")
                config["output_dir"] = str(base / "out2")
                config["command"] = [sys.executable, "-c", "from pathlib import Path; "
                                     "Path('a').write_bytes(b'x'*60000); Path('b').write_bytes(b'x'*60000); "
                                     "import time; time.sleep(4)"]
                result = run_cpu.run(config, ready)
                self.assertEqual(result["status"], "scratch_cap_exceeded")
                self.assertEqual(result["quality_acceptance"], "not_evaluated")

    def test_kaggle_requires_matching_account_and_stages_private(self):
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            harness = base / "kaggle_harness.py"
            harness.write_text("def init_progress(**kwargs): pass\n")
            config = {"account": "tester", "slug": "crispmesh-quality-r1", "run_id": "quality-r1",
                      "input_dataset": "other/input", "archive_name": "quality.zip",
                      "command": ["/usr/bin/python3", "scripts/example.py"]}
            with self.assertRaisesRegex(ValueError, "owned"):
                prepare(config, base / "out", harness)
            config["input_dataset"] = "tester/input"
            result = prepare(config, base / "out", harness)
            self.assertEqual(result["launch"], "disabled")
            metadata = json.loads((base / "out/kernel-metadata.json").read_text())
            self.assertEqual(metadata["is_private"], "true")
            self.assertEqual(metadata["dataset_sources"], ["tester/input"])
            code = (base / "out/remote_quality.py").read_text()
            self.assertIn("quality-r1", code)
            compile(code, "remote_quality.py", "exec")


if __name__ == "__main__":
    unittest.main()
