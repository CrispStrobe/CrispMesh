import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import numpy as np

from scripts.object_motion import sam_m1_parity as parity
from scripts.object_motion import ycb_object_masks as common


class SAMM1ParityTests(unittest.TestCase):
    def test_darwin_memory_and_rss_parsers_fail_closed(self):
        sample = """Mach Virtual Memory Statistics: (page size of 16384 bytes)
Pages free: 100,000.
Pages inactive: 60,000.
Pages speculative: 4,000.
"""
        self.assertEqual(parity.darwin_available_kib(sample), 160000 * 16)
        self.assertEqual(parity.darwin_rss_kib(" 1536000\n"), 1536000)
        for broken in (sample.replace("Pages inactive", "Other"), sample.replace("16384", "512")):
            with self.assertRaises(ValueError):
                parity.darwin_available_kib(broken)
        with self.assertRaises(ValueError):
            parity.darwin_rss_kib("not-a-number")

    def test_exact_three_frozen_names_and_correct_reference_batches(self):
        path = Path(__file__).parents[2] / "tests" / "datasets" / "sam21_mustard_point48_prompts.json"
        data = json.loads(path.read_text())["images"]
        rows = [{"path": "photos/" + item["name"], "sha256": item["source_sha256"]} for item in data]
        sealed = [{"name": item["name"]} for item in data[:16]]
        with (patch.object(parity.full, "validated_inputs", return_value=(rows, data, "a" * 64)),
              patch.object(parity.resume, "prior_sealed_rows", return_value=sealed)):
            selected, _ = parity.selected_inputs("package", "prompts", "checkpoint", "source", "prior")
        self.assertEqual([(Path(row["path"]).name, batch) for row, _, _, batch in selected],
                         [("NP3_000.jpg", 0), ("NP3_066.jpg", 1), ("NP3_108.jpg", 1)])
        self.assertEqual(common.digest(path), parity.full.PROMPTS_SHA)

    def test_binary_iou_and_frozen_parity_gate(self):
        left = np.array([[255, 255], [0, 0]], dtype=np.uint8)
        right = np.array([[255, 0], [0, 0]], dtype=np.uint8)
        same = parity.overlap(left, left)
        changed = parity.overlap(left, right)
        self.assertEqual(same["iou"], 1.0)
        self.assertEqual(changed["iou"], 0.5)
        self.assertTrue(same["identical_pixels"])
        self.assertFalse(changed["identical_pixels"])
        self.assertTrue(parity.parity_gate({"iou": 0.99}, [1, 0, 0], [1, 0, 0]))
        self.assertFalse(parity.parity_gate({"iou": 0.98999}, [1, 0, 0], [1, 0, 0]))
        self.assertFalse(parity.parity_gate({"iou": 1.0}, [1, 1, 0], [1, 0, 0]))
        with self.assertRaises(ValueError):
            parity.overlap(left.astype(np.float32), right)

    def test_all_terminal_caps_and_allowed_boundaries(self):
        allowed = (parity.MAX_SECONDS, parity.MAX_RSS_KIB, parity.MIN_AVAILABLE_KIB,
                   parity.MAX_OUTPUT, parity.MAX_LOG, common.MIN_FREE_BYTES, 0)
        self.assertIsNone(parity.terminal_guard(*allowed))
        changes = [(0, parity.MAX_SECONDS + 0.01), (1, parity.MAX_RSS_KIB + 1),
                   (2, parity.MIN_AVAILABLE_KIB - 1), (3, parity.MAX_OUTPUT + 1),
                   (4, parity.MAX_LOG + 1), (5, common.MIN_FREE_BYTES - 1), (6, 1)]
        for index, value in changes:
            case = list(allowed)
            case[index] = value
            with self.subTest(index=index):
                self.assertIsNotNone(parity.terminal_guard(*case))

    def test_external_mount_and_internal_disk_gate(self):
        workspace = Path(parity.__file__).resolve().parents[2]
        def fake_stat(path, **_kwargs):
            value = str(path).replace("\\", "/")
            workspace_text = str(workspace).replace("\\", "/")
            device = 1 if value == workspace_text or "/Users/" in value else 2
            return type("S", (), {"st_dev": device, "st_mode": 0o040755})()
        with (patch.object(parity.sys, "platform", "darwin"),
              patch.object(parity.os, "uname", return_value=type("U", (), {"machine": "arm64"})(), create=True),
              patch.object(parity.Path, "is_dir", return_value=True),
              patch.object(parity.Path, "is_mount", return_value=True),
              patch.object(parity.Path, "is_symlink", return_value=False),
              patch.object(parity.os, "stat", side_effect=fake_stat),
              patch.object(parity, "disk_free_both", return_value=common.MIN_FREE_BYTES),
              patch.object(parity, "available_kib", return_value=parity.MIN_AVAILABLE_KIB)):
            parity.host_preflight(Path("/Volumes/backups/code/crispmesh-data/out"))
            with self.assertRaisesRegex(ValueError, "external"):
                parity.host_preflight(Path("/Users/example/internal"))
            with patch.object(parity.Path, "is_mount", return_value=False):
                with self.assertRaisesRegex(ValueError, "external"):
                    parity.host_preflight(Path("/Volumes/backups/code/crispmesh-data/out"))

    def test_missing_manifest_and_nonexternal_inputs_fail_closed(self):
        status, reason = parity.manifest_status(Path("/nonexistent/mac-sam-parity.json"))
        self.assertIsNone(status)
        self.assertIn("without valid", reason)
        with tempfile.TemporaryDirectory(dir=Path.cwd()) as directory:
            path = Path(directory) / "manifest.json"
            path.write_text("{broken")
            status, reason = parity.manifest_status(path)
            self.assertIsNone(status)
            self.assertIn("malformed", reason)
        with (patch.object(parity.Path, "is_mount", return_value=True),
              patch.object(parity.Path, "stat", return_value=type("S", (), {"st_dev": 2, "st_mode": 0o100644})())):
            with self.assertRaisesRegex(ValueError, "external volume"):
                parity.external_inputs((Path(__file__),))

    def test_failed_membership_preserves_raw_mask_and_failure_manifest(self):
        row = {"path": "photos/NP3_000.jpg", "sha256": "a" * 64}
        prompt = {"points_xy_label": [[600, 525, 1], [720, 620, 0], [520, 630, 0]]}
        selected = [(row, prompt, {"name": "NP3_000.jpg"}, 0)]
        with tempfile.TemporaryDirectory(dir=Path.cwd()) as directory:
            output = Path(directory) / "fresh"
            with (patch.object(parity, "host_preflight"),
                  patch.object(parity, "external_inputs"),
                  patch.object(parity, "selected_inputs", return_value=(selected, "b" * 64)),
                  patch.object(parity.point, "sam_predictor", return_value=lambda *_: np.zeros((1024, 1280), dtype=np.uint8)),
                  patch.object(parity.common, "verified_training_photo", return_value=None),
                  patch.object(parity, "available_kib", return_value=parity.MIN_AVAILABLE_KIB),
                  patch.object(parity, "rss_kib", return_value=0),
                  patch.object(parity, "disk_free_both", return_value=common.MIN_FREE_BYTES)):
                with self.assertRaisesRegex(ValueError, "point labels"):
                    parity.worker("package", "photos", "prompts", "checkpoint", "source", "prior", output)
            raw = output / "raw_masks" / "NP3_000.jpg.png"
            self.assertTrue(raw.is_file())
            report = json.loads((output / "manifest.json").read_text())
            self.assertEqual(report["status"], "failed")
            self.assertEqual(report["images"][0]["raw_mask_sha256"], common.digest(raw))


if __name__ == "__main__":
    unittest.main()
