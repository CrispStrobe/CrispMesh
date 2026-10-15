"""Hand-computed CLI checks for the opt-in stereo evaluator.

Run: CRISPMESH_TEST_STEREO_EVAL=1 python3 -m unittest scripts/test_stereo_eval_cli.py
Requires the OpenCV build target build-opencv/bin/crispmesh_stereo_eval.
"""

import json
import math
import os
from pathlib import Path
import struct
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent.parent
CLI = ROOT / "build-opencv/bin/crispmesh_stereo_eval"


def write_pfm(path: Path, rows: list[list[float]], scale: float = -1.0) -> None:
    """Write grayscale PFM; scale sign selects endian, disk rows are bottom first."""
    height = len(rows)
    width = len(rows[0])
    assert height and width and all(len(row) == width for row in rows)
    with path.open("wb") as output:
        output.write(f"Pf\n{width} {height}\n{scale}\n".encode("ascii"))
        for row in reversed(rows):
            output.write(struct.pack(f"{'<' if scale < 0 else '>'}{width}f", *row))


def read_pfm(path: Path) -> list[list[float]]:
    with path.open("rb") as source:
        assert source.readline() == b"Pf\n"
        width, height = map(int, source.readline().split())
        assert source.readline() == b"-1.0\n"
        payload = source.read()
    assert len(payload) == width * height * 4
    values = struct.unpack(f"<{width * height}f", payload)
    rows = [list(values[i * width:(i + 1) * width]) for i in range(height)]
    return list(reversed(rows))


@unittest.skipUnless(os.environ.get("CRISPMESH_TEST_STEREO_EVAL") == "1", "opt-in evaluator CLI checks")
class StereoEvalCliTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        if not CLI.is_file():
            raise AssertionError(f"Build the evaluator first: cmake --build build-opencv --target crispmesh_stereo_eval ({CLI})")

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="crispmesh-stereo-cli-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.truth = self.root / "truth.pfm"
        self.prediction = self.root / "prediction.pfm"
        write_pfm(self.truth, [[0, 3, 4], [math.inf, 9, -1]])
        write_pfm(self.prediction, [[0, 6, math.inf], [1, 9, 8]])

    def run_cli(self, output: Path, *extra: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [str(CLI), "--gt", str(self.truth), "--prediction", str(self.prediction),
             "--output-dir", str(output), *extra],
            text=True, capture_output=True, check=False, timeout=15,
        )

    def test_hand_computed_coverage_bad2_and_zero_disparity(self) -> None:
        output = self.root / "metrics-run"
        result = self.run_cli(output)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), str(output / "metrics.json"))
        metrics = json.loads((output / "metrics.json").read_text())
        self.assertEqual((metrics["source_width"], metrics["source_height"]), (3, 2))
        self.assertEqual(metrics["mode"], "prediction")
        self.assertEqual(metrics["mask_role"], "evaluation_only")
        self.assertEqual((metrics["roi_pixels"], metrics["gt_valid"], metrics["matched"]), (6, 4, 3))
        self.assertEqual(metrics["bad2_matched_count"], 1)
        self.assertAlmostEqual(metrics["coverage"], 3 / 4)
        self.assertAlmostEqual(metrics["bad2_all_valid"], 2 / 4)  # one bad match + one missing
        self.assertAlmostEqual(metrics["bad2_matched"], 1 / 3)
        self.assertAlmostEqual(metrics["mae_matched_px"], 1)  # errors 0, 3, 0
        self.assertAlmostEqual(metrics["rmse_matched_px"], math.sqrt(3))
        output_pixels = read_pfm(output / "disparity.pfm")
        self.assertEqual(output_pixels[0][0], 0)  # valid zero disparity survives PFM roundtrip
        self.assertTrue(math.isinf(output_pixels[0][2]))
        self.assertFalse((output / "depth.pfm").exists())  # no calibration was supplied

    def test_explicit_depth_formula_including_zero_disparity(self) -> None:
        output = self.root / "depth-run"
        result = self.run_cli(output, "--fx", "10", "--baseline", "5", "--doffs", "3", "--ndisp", "16")
        self.assertEqual(result.returncode, 0, result.stderr)
        metrics = json.loads((output / "metrics.json").read_text())
        self.assertEqual(metrics["calibration"], {"fx": 10, "baseline": 5, "doffs": 3, "ndisp": 16})
        self.assertEqual(metrics["depth_formula"], "Z=fx*baseline/(d+doffs)")
        depth = read_pfm(output / "depth.pfm")
        self.assertAlmostEqual(depth[0][0], 50 / 3, places=5)
        self.assertAlmostEqual(depth[0][1], 50 / 9, places=5)
        self.assertTrue(math.isinf(depth[0][2]))
        self.assertAlmostEqual(depth[1][1], 50 / 12, places=5)

    def test_no_matches_has_null_matched_error_metrics(self) -> None:
        write_pfm(self.prediction, [[math.inf] * 3, [math.inf] * 3])
        output = self.root / "no-match-run"
        result = self.run_cli(output)
        self.assertEqual(result.returncode, 0, result.stderr)
        metrics = json.loads((output / "metrics.json").read_text())
        self.assertEqual(metrics["gt_valid"], 4)
        self.assertEqual(metrics["matched"], 0)
        self.assertEqual(metrics["coverage"], 0)
        self.assertEqual(metrics["bad2_all_valid"], 1)
        self.assertIsNone(metrics["bad2_matched"])
        self.assertIsNone(metrics["mae_matched_px"])
        self.assertIsNone(metrics["rmse_matched_px"])

    def test_pfm_scale_magnitude_is_not_applied_and_sign_selects_endianness(self) -> None:
        truth_rows = [[0, 3, 4], [math.inf, 9, -1]]
        prediction_rows = [[0, 6, math.inf], [1, 9, 8]]
        for truth_scale, prediction_scale in ((-0.003922, 2.0), (2.0, -0.003922)):
            with self.subTest(truth_scale=truth_scale, prediction_scale=prediction_scale):
                write_pfm(self.truth, truth_rows, truth_scale)
                write_pfm(self.prediction, prediction_rows, prediction_scale)
                output = self.root / f"scale-{truth_scale}-{prediction_scale}"
                result = self.run_cli(output)
                self.assertEqual(result.returncode, 0, result.stderr)
                metrics = json.loads((output / "metrics.json").read_text())
                self.assertEqual((metrics["gt_valid"], metrics["matched"], metrics["bad2_matched_count"]), (4, 3, 1))
                self.assertAlmostEqual(metrics["mae_matched_px"], 1)
                self.assertEqual(read_pfm(output / "disparity.pfm")[0][1], 6)

    def test_rejects_bad_downsample_and_zero_valid_ground_truth(self) -> None:
        for bad in ("1.5", "1e100"):
            with self.subTest(downsample=bad):
                output = self.root / f"bad-downsample-{bad}"
                result = self.run_cli(output, "--downsample", bad)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("downsample", result.stderr.lower())
                self.assertFalse(output.exists())
        write_pfm(self.truth, [[-1, math.inf, math.nan], [-2, math.inf, -3]])
        output = self.root / "zero-gt"
        result = self.run_cli(output)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("No valid ground-truth", result.stderr)
        self.assertFalse(output.exists())

    def test_existing_output_collision_preserves_input_and_metrics(self) -> None:
        output = self.root / "collision"
        output.mkdir()
        prediction_in_output = output / "disparity.pfm"
        prediction_in_output.write_bytes(self.prediction.read_bytes())
        before_prediction = prediction_in_output.read_bytes()
        result = subprocess.run(
            [str(CLI), "--gt", str(self.truth), "--prediction", str(prediction_in_output),
             "--output-dir", str(output)],
            text=True, capture_output=True, check=False, timeout=15,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Output already exists", result.stderr)
        self.assertEqual(prediction_in_output.read_bytes(), before_prediction)
        self.assertFalse((output / "metrics.json").exists())
        metrics_collision = self.root / "metrics-collision"
        metrics_collision.mkdir()
        sentinel = b"keep previous metrics exactly\n"
        (metrics_collision / "metrics.json").write_bytes(sentinel)
        result = self.run_cli(metrics_collision)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Output already exists", result.stderr)
        self.assertEqual((metrics_collision / "metrics.json").read_bytes(), sentinel)
        self.assertFalse((metrics_collision / "disparity.pfm").exists())


if __name__ == "__main__":
    unittest.main()
