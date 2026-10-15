"""Offline preflight tests for the private three-photo AliceVision sparse smoke."""

from __future__ import annotations

import ast
import json
from pathlib import Path
import sys
import tempfile
import unittest


FOLDER = Path(__file__).resolve().parent / "alicevision_three_photo_smoke"
sys.path.insert(0, str(FOLDER))
import alicevision_three_photo_smoke as smoke  # noqa: E402


class ThreePhotoSmokeTests(unittest.TestCase):
    def test_private_cpu_only_exact_dataset(self) -> None:
        metadata = json.loads((FOLDER / "kernel-metadata.json").read_text())
        self.assertEqual(metadata["is_private"], "true")
        self.assertEqual(metadata["enable_gpu"], "false")
        self.assertEqual(metadata["dataset_sources"],
                         ["chr1str/crispmesh-bunny-3photo-research-smoke"])
        for key in ("competition_sources", "kernel_sources", "model_sources"):
            self.assertEqual(metadata[key], [])

    def test_selected_photos_match_sealed_prepare_manifest(self) -> None:
        prep = (Path(__file__).resolve().parents[2] / "build-opencv" /
                "bunny-gamma05-clahe2" / "prepare-manifest.json")
        items = {item["output"]: item for item in json.loads(prep.read_text())["images"]}
        self.assertEqual(list(smoke.PHOTOS),
                         ["frame_0000.png", "frame_0022.png", "frame_0044.png"])
        for filename, (_, digest) in smoke.PHOTOS.items():
            self.assertEqual(items[filename]["output_sha256"], digest)
        self.assertEqual([items[filename]["source"] for filename in smoke.PHOTOS],
                         ["bunny_0_rgb.png", "bunny_2_rgb.png", "bunny_4_rgb.png"])

    def test_kernel_entry_is_self_contained(self) -> None:
        imports = [node.module for node in ast.walk(ast.parse((FOLDER / "alicevision_three_photo_smoke.py").read_text()))
                   if isinstance(node, ast.ImportFrom)]
        imports += [alias.name for node in ast.walk(ast.parse((FOLDER / "alicevision_three_photo_smoke.py").read_text()))
                    if isinstance(node, ast.Import) for alias in node.names]
        self.assertNotIn("alicevision_binary_smoke", imports)
        self.assertEqual(sorted(path.name for path in FOLDER.glob("*.py")),
                         ["alicevision_three_photo_smoke.py"])

    def test_work_cap_and_locations(self) -> None:
        self.assertEqual(smoke.MAX_SECONDS, 1200)
        self.assertEqual(smoke.MAX_WORK_BYTES, 1 << 30)
        self.assertEqual(smoke.MIN_FREE_AFTER, 4 << 30)
        self.assertEqual(smoke.SCRATCH.parent, Path("/tmp"))
        self.assertEqual(smoke.OUTPUT.parent, Path("/kaggle/working"))
        self.assertEqual(smoke.ARCHIVE_BYTES, 1_505_191_867)
        with tempfile.TemporaryDirectory() as directory:
            (Path(directory) / "test.bin").write_bytes(b"1234")
            self.assertEqual(smoke.work_bytes(Path(directory)), 4)

    def test_dataset_mount_resolver_rejects_missing_and_ambiguous(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            first = Path(directory) / "first"
            second = Path(directory) / "second"
            with self.assertRaisesRegex(RuntimeError, "missing or ambiguous"):
                smoke.resolve_dataset((first, second))
            first.mkdir()
            self.assertEqual(smoke.resolve_dataset((first, second)), first)
            second.mkdir()
            with self.assertRaisesRegex(RuntimeError, "missing or ambiguous"):
                smoke.resolve_dataset((first, second))


if __name__ == "__main__":
    unittest.main()
