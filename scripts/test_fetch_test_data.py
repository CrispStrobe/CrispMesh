import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import struct
import tempfile
import unittest
import zlib


SCRIPT = Path(__file__).with_name("fetch_test_data.py")
spec = importlib.util.spec_from_file_location("fetch_test_data", SCRIPT)
fetch = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fetch)
LOCAL_TMP = fetch.REPO / ".local-tools/tmp"
LOCAL_TMP.mkdir(parents=True, exist_ok=True)


class FetchTestDataTests(unittest.TestCase):
    def test_committed_manifest_is_bounded_and_pinned(self):
        manifest = json.loads(fetch.MANIFEST.read_text())
        self.assertFalse(manifest["upstream_equivalence_verified"])
        self.assertEqual(len(fetch.validate_manifest(manifest)), 4)

    def test_manifest_rejects_traversal_and_unpinned_url(self):
        manifest = json.loads(fetch.MANIFEST.read_text())
        manifest["files"][0]["path"] = "../calib.txt"
        with self.assertRaises(ValueError):
            fetch.validate_manifest(manifest)
        manifest = json.loads(fetch.MANIFEST.read_text())
        manifest["files"][0]["url"] = "http://example.com/calib.txt"
        with self.assertRaises(ValueError):
            fetch.validate_manifest(manifest)

    def test_existing_file_requires_exact_hash_and_rejects_symlink(self):
        with tempfile.TemporaryDirectory(dir=LOCAL_TMP) as directory:
            path = Path(directory) / "sample"
            path.write_bytes(b"measured")
            entry = {"size": 8, "sha256": hashlib.sha256(b"measured").hexdigest()}
            self.assertTrue(fetch.file_matches(path, entry))
            path.write_bytes(b"modified")
            self.assertFalse(fetch.file_matches(path, entry))
            path.unlink()
            path.symlink_to(SCRIPT)
            self.assertFalse(fetch.file_matches(path, entry))

    def test_calibration_parser(self):
        data = (b"cam0=[10 0 5; 0 10 4; 0 0 1]\n"
                b"cam1=[10 0 7; 0 10 4; 0 0 1]\n"
                b"doffs=2\nbaseline=50\nwidth=8\nheight=6\nndisp=4\n")
        result = fetch.parse_calibration(data)
        self.assertEqual((result["fx"], result["baseline_mm"], result["doffs"]), (10, 50, 2))
        with self.assertRaises(ValueError):
            fetch.parse_calibration(data.replace(b"doffs=2", b"doffs=3"))
        with self.assertRaises(ValueError):
            fetch.parse_calibration(data.replace(b"baseline=50", b"baseline=nan"))
        with self.assertRaises(ValueError):
            fetch.parse_calibration(data.replace(b"cam0=[10", b"cam0=[inf"))

    def test_pfm_header_and_payload_length(self):
        with tempfile.TemporaryDirectory(dir=LOCAL_TMP) as directory:
            path = Path(directory) / "disp.pfm"
            path.write_bytes(b"Pf\n2 2\n-1.0\n" + struct.pack("<4f", 1, 2, 3, 4))
            self.assertEqual(fetch.read_pfm_header(path)["width"], 2)
            self.assertTrue(fetch.read_pfm_header(path)["little_endian"])
            path.write_bytes(path.read_bytes()[:-1])
            with self.assertRaises(ValueError):
                fetch.read_pfm_header(path)
            path.write_bytes(b"Pf\n2 2\n-0.003922\n" + struct.pack("<4f", 1, 2, 3, 4))
            self.assertTrue(fetch.read_pfm_header(path)["little_endian"])
            path.write_bytes(b"Pf\n2 2\nnan\n" + struct.pack("<4f", 1, 2, 3, 4))
            with self.assertRaises(ValueError):
                fetch.read_pfm_header(path)

    def test_additional_manifests_reject_unexpected_paths_and_sizes(self):
        old = json.loads(fetch.MANIFEST_2001.read_text())
        self.assertEqual(len(fetch.validate_2001_manifest(old)), 3)
        old["files"][0]["path"] = "../im2.png"
        with self.assertRaises(ValueError):
            fetch.validate_2001_manifest(old)
        year_2003 = json.loads(fetch.MANIFEST_2003.read_text())
        self.assertEqual(len(fetch.validate_2003_manifest(year_2003)), 6)
        year_2003["derived"][0]["divisor"] = 8
        with self.assertRaises(ValueError):
            fetch.validate_2003_manifest(year_2003)
        eth = json.loads(fetch.MANIFEST_ETH3D.read_text())
        self.assertEqual(len(fetch.validate_eth3d_manifest(eth)[1]), 15)
        eth["files"][0]["path"] = "../calib.txt"
        with self.assertRaises(ValueError):
            fetch.validate_eth3d_manifest(eth)

    def test_gray_png_to_pfm_scale_and_invalid_convention(self):
        width, height = 450, 375
        rows = [bytes([0, 8]) + bytes(width - 2)] * height
        pixels = b"".join(b"\x00" + row for row in rows)
        def chunk(kind, content):
            return (struct.pack(">I", len(content)) + kind + content
                    + struct.pack(">I", zlib.crc32(kind + content)))
        header = struct.pack(">IIBBBBB", width, height, 8, 0, 0, 0, 0)
        png = (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header)
               + chunk(b"IDAT", zlib.compress(pixels)) + chunk(b"IEND", b""))
        pfm = fetch.disparity_pfm_bytes(png)
        self.assertEqual(pfm[:16], b"Pf\n450 375\n-1.0\n")
        self.assertEqual(struct.unpack_from("<2f", pfm, 16), (float("inf"), 2.0))
        with self.assertRaises(ValueError):
            fetch.read_gray8_png(png[:-1])

    def test_oversized_download_is_rejected_without_target(self):
        class FakeOpener:
            def open(self, *_args, **_kwargs):
                return io.BytesIO(b"too big")
        with tempfile.TemporaryDirectory(dir=LOCAL_TMP) as directory:
            root = Path(directory)
            entry = {"path": "sample", "url": "https://raw.githubusercontent.com/example",
                     "size": 3, "sha256": "0" * 64}
            with self.assertRaises(ValueError):
                fetch.fetch_one(root, entry, FakeOpener())
            self.assertFalse((root / "sample").exists())

    def test_local_measured_fixture_when_requested(self):
        if os.getenv("CRISPMESH_TEST_REAL_DATA") != "1":
            self.skipTest("set CRISPMESH_TEST_REAL_DATA=1 after --fetch")
        piano_root = fetch.selected_root("piano")
        entries = fetch.validate_manifest(json.loads(fetch.MANIFEST.read_text()))
        for entry in entries:
            self.assertTrue(fetch.file_matches(piano_root / entry["path"], entry))
        scene = fetch.verify_scene(piano_root)
        self.assertEqual((scene["width"], scene["height"]), (2820, 1920))
        self.assertEqual(scene["baseline_mm"], 178.089)
        with (piano_root / "Piano-perfect/disp0.pfm").open("rb") as disparity:
            self.assertEqual(disparity.readline().strip(), b"Pf")
            disparity.readline()
            self.assertEqual(disparity.readline().strip(), b"-0.003922")
        for group, scenes, validator, manifest_path in (
                ("middlebury2001", ("venus",), fetch.validate_2001_manifest, fetch.MANIFEST_2001),
                ("middlebury2003", ("cones", "teddy"), fetch.validate_2003_manifest, fetch.MANIFEST_2003)):
            root = fetch.selected_root(group)
            manifest = json.loads(manifest_path.read_text())
            for entry in validator(manifest):
                self.assertTrue(fetch.file_matches(root / entry["path"], entry))
            for entry in manifest["derived"]:
                self.assertEqual(fetch.verify_or_create_derived(root, entry), "verified")
            for name in scenes:
                fetch.verify_legacy_scene(root, name, (434, 383) if name == "venus" else (450, 375))

    def test_eth3d_fixture_when_requested(self):
        if os.getenv("CRISPMESH_TEST_ETH3D") != "1":
            self.skipTest("set CRISPMESH_TEST_ETH3D=1 for noncommercial ETH3D local data")
        root = fetch.selected_root("eth3d")
        archives, files = fetch.validate_eth3d_manifest(json.loads(fetch.MANIFEST_ETH3D.read_text()))
        for entry in archives:
            self.assertTrue(fetch.file_matches(root / entry["name"], entry))
        for entry in files:
            self.assertTrue(fetch.file_matches(root / entry["path"], entry))
        for scene in ("delivery_area_1s", "forest_1s", "playground_1s"):
            self.assertGreater(fetch.verify_eth3d_scene(root, scene)["baseline_mm"], 0)


if __name__ == "__main__":
    unittest.main()
