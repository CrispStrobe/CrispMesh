"""Unit and CLI checks for profile aggregation and default parity."""

import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from scripts.compare_stereo_profiles import PROFILES, main, summarize
from scripts.test_stereo_eval_cli import write_pfm

ROOT = Path(__file__).resolve().parent.parent
CLI = ROOT / "build-opencv/bin/crispmesh_stereo_eval"


class CompareProfilesTest(unittest.TestCase):
    def test_fixed_profiles_and_median(self):
        self.assertEqual(PROFILES, ("baseline", "quality", "fast", "full"))
        samples = []
        for value in (1.0, 3.0, 2.0):
            samples.append({"coverage": value / 3, "bad2_all_valid": value / 10,
                            "bad2_matched": None, "mae_matched_px": value,
                            "elapsed_stereo_ms": value * 10,
                            "stage_ms": {key: value for key in
                                         ("decode", "preprocess", "match", "filter", "score")}})
        summary = summarize(samples)
        self.assertEqual(summary["elapsed_stereo_ms_median"], 20)
        self.assertEqual(summary["match_ms_median"], 2)
        self.assertIsNone(summary["bad2_matched_median"])
        with self.assertRaises(ValueError):
            summarize([])

    @unittest.skipUnless(CLI.is_file(), "build evaluator first")
    def test_default_and_explicit_baseline_disparity_parity(self):
        with tempfile.TemporaryDirectory(prefix="crispmesh-profile-") as folder:
            root = Path(folder)
            rows = [[(x * 57 + y * 113 + x * y * 19) & 255 for x in range(128)] for y in range(48)]
            for name, pixels in (("left.pgm", rows),
                                 ("right.pgm", [[row[min(x + 8, 127)] for x in range(128)] for row in rows])):
                (root / name).write_bytes(b"P5\n128 48\n255\n" + bytes(v for row in pixels for v in row))
            write_pfm(root / "truth.pfm", [[8.0] * 128 for _ in range(48)])
            common = [str(CLI), "--left", str(root / "left.pgm"), "--right", str(root / "right.pgm"),
                      "--gt", str(root / "truth.pfm"), "--ndisp", "16"]
            for name, extra in (("default", []), ("explicit", ["--profile", "baseline"])):
                run = subprocess.run(common + extra + ["--output-dir", str(root / name)],
                                     text=True, capture_output=True, timeout=30)
                self.assertEqual(run.returncode, 0, run.stderr)
            self.assertEqual((root / "default/disparity.pfm").read_bytes(),
                             (root / "explicit/disparity.pfm").read_bytes())
            default = json.loads((root / "default/metrics.json").read_text())
            explicit = json.loads((root / "explicit/metrics.json").read_text())
            self.assertNotIn("profile", default)
            self.assertEqual(explicit["profile"], "baseline")
            self.assertEqual(default["sgbm"], explicit["sgbm"])
            self.assertEqual(explicit["sgbm"]["mode"], "SGBM")
            self.assertEqual(explicit["sgbm"]["P1"], 200)
            self.assertEqual(explicit["sgbm"]["P2"], 800)
            self.assertIn("stage_ms", explicit)

    @unittest.skipUnless(CLI.is_file(), "build evaluator first")
    def test_failed_trial_persists_summary(self):
        with tempfile.TemporaryDirectory(prefix="crispmesh-profile-failure-") as folder:
            fake_scene = {"manifest": {"width": 128, "height": 48, "calibration": None},
                          "manifest_sha256": "0", "prepared_sha256": {}, "sources": {},
                          "ndisp": 16, "common_search_count": 16,
                          "files": {"left": Path(folder) / "left", "right": Path(folder) / "right",
                                    "truth": Path(folder) / "truth"}}
            with patch("scripts.compare_stereo_profiles.read_prepared", return_value=fake_scene), \
                 patch("scripts.compare_stereo_profiles.check_unchanged"), \
                 patch("scripts.compare_stereo_profiles.execute",
                       return_value={"status": "failed", "exit_code": 9, "log": "fake.log"}):
                code = main(["--prepared", "fixture=" + folder, "--evaluator", str(CLI),
                             "--output-root", folder, "--warmup", "0", "--repeats", "1"])
            self.assertEqual(code, 1)
            summary = json.loads(next(Path(folder).glob("run-*/summary.json")).read_text())
            self.assertFalse(summary["ok"])
            self.assertEqual(summary["scenes"]["fixture"]["profiles"]["baseline"]["trials"][0]["exit_code"], 9)


if __name__ == "__main__":
    unittest.main()
