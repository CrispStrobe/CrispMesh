"""Small subprocess checks for the local research guard."""

from __future__ import annotations

from contextlib import contextmanager
from unittest.mock import patch
import os
import json
import math
from pathlib import Path
import sys
import tempfile
import time
import unittest

from scripts.research_job.guard import TEMP_ROOT, run_child, _tree_bytes


class GuardTests(unittest.TestCase):
    def setUp(self) -> None:
        TEMP_ROOT.mkdir(parents=True, exist_ok=True)

    def test_files_released_between_enumeration_and_stat_are_not_guard_failures(self) -> None:
        with tempfile.TemporaryDirectory(dir=TEMP_ROOT) as root:
            base = Path(root)
            released = base / "intermediate.png"
            released.write_bytes(b"temporary")
            (base / "result.stl").write_bytes(b"mesh")
            real_scandir = os.scandir

            @contextmanager
            def scan(path):
                with real_scandir(path) as entries:
                    rows = list(entries)
                released.unlink()
                yield iter(rows)

            with patch("scripts.research_job.guard.os.scandir", side_effect=scan):
                self.assertEqual(_tree_bytes(base), 4)

    def test_directory_released_after_it_is_queued_is_not_a_guard_failure(self) -> None:
        with tempfile.TemporaryDirectory(dir=TEMP_ROOT) as root:
            base = Path(root)
            released = base / "intermediates"
            released.mkdir()
            (base / "result.stl").write_bytes(b"mesh")
            real_scandir = os.scandir

            def scan(path):
                if Path(path) == released:
                    released.rmdir()
                return real_scandir(path)

            with patch("scripts.research_job.guard.os.scandir", side_effect=scan):
                self.assertEqual(_tree_bytes(base), 4)

    def test_success_and_input_unchanged(self) -> None:
        with tempfile.TemporaryDirectory(dir=TEMP_ROOT) as root:
            base = Path(root)
            source = base / "input.txt"
            source.write_text("original")
            output = base / "run"
            code = "from pathlib import Path; print(Path('input.txt').read_text())"
            status = run_child([sys.executable, "-c", code], base, output,
                               reserve_bytes=0, max_output_bytes=100_000)
            self.assertEqual(status["status"], "succeeded")
            self.assertEqual(source.read_text(), "original")
            self.assertIn("original", (output / "child.log").read_text())
            self.assertEqual(json.loads((output / "status.json").read_text()), status)
            with self.assertRaisesRegex(ValueError, "fresh"):
                run_child([sys.executable, "-c", "pass"], base, output, reserve_bytes=0)

    def test_output_cap_and_log_cap(self) -> None:
        with tempfile.TemporaryDirectory(dir=TEMP_ROOT) as root:
            base = Path(root)
            output = base / "output"
            code = "from pathlib import Path; Path('output/data').write_bytes(b'x'*4096)"
            result = run_child([sys.executable, "-c", code], base, output,
                               reserve_bytes=0, max_output_bytes=1024)
            self.assertEqual(result["status"], "output_cap_exceeded")
            log = base / "log"
            result = run_child([sys.executable, "-c", "print('x'*4096)"], base, log,
                               reserve_bytes=0, max_log_bytes=128)
            self.assertEqual(result["status"], "log_cap_exceeded")

    def test_timeout_kills_descendant(self) -> None:
        with tempfile.TemporaryDirectory(dir=TEMP_ROOT) as root:
            base = Path(root)
            marker = base / "late.txt"
            child = ("import subprocess,sys,time; "
                     "subprocess.Popen([sys.executable,'-c',"
                     "'import time,pathlib; time.sleep(1); pathlib.Path(sys.argv[1]).write_text(\"late\")',"
                     "sys.argv[1]]); time.sleep(2)")
            output = base / "run"
            result = run_child([sys.executable, "-c", child, str(marker)], base, output,
                               timeout_seconds=0.2, reserve_bytes=0)
            self.assertEqual(result["status"], "timeout")
            time.sleep(1.1)
            self.assertFalse(marker.exists())

    def test_normal_leader_exit_kills_descendant(self) -> None:
        with tempfile.TemporaryDirectory(dir=TEMP_ROOT) as root:
            base = Path(root)
            marker = base / "late.txt"
            child = ("import subprocess,sys; "
                     "subprocess.Popen([sys.executable,'-c',"
                     "'import time,pathlib,sys; time.sleep(1); pathlib.Path(sys.argv[1]).write_text(\"late\")',"
                     "sys.argv[1]])")
            result = run_child([sys.executable, "-c", child, str(marker)], base,
                               base / "run", reserve_bytes=0)
            self.assertEqual(result["status"], "succeeded")
            time.sleep(1.1)
            self.assertFalse(marker.exists())

    def test_nonfinite_timeout_and_fractional_caps_rejected(self) -> None:
        with tempfile.TemporaryDirectory(dir=TEMP_ROOT) as root:
            base = Path(root)
            for bad in (math.nan, math.inf, -math.inf):
                with self.assertRaisesRegex(ValueError, "finite"):
                    run_child([sys.executable], base, base / "run",
                              timeout_seconds=bad, reserve_bytes=0)
            with self.assertRaisesRegex(ValueError, "integer"):
                run_child([sys.executable], base, base / "run",
                          max_output_bytes=1.5, reserve_bytes=0)
            self.assertFalse((base / "run").exists())

    def test_launch_failure_retains_status(self) -> None:
        with tempfile.TemporaryDirectory(dir=TEMP_ROOT) as root:
            base = Path(root)
            output = base / "run"
            result = run_child([str(base / "missing-executable")], base, output,
                               reserve_bytes=0)
            self.assertEqual(result["status"], "guard_error")
            self.assertEqual(result["error_type"], "FileNotFoundError")
            self.assertEqual(json.loads((output / "status.json").read_text()), result)

    def test_symlink_output_rejected(self) -> None:
        with tempfile.TemporaryDirectory(dir=TEMP_ROOT) as root:
            base = Path(root)
            target = base / "existing"
            target.mkdir()
            link = base / "linked"
            link.symlink_to(target, target_is_directory=True)
            with self.assertRaisesRegex(ValueError, "symlink"):
                run_child([sys.executable, "-c", "pass"], base, link,
                          reserve_bytes=0)
            self.assertEqual(list(target.iterdir()), [])


if __name__ == "__main__":
    unittest.main()
