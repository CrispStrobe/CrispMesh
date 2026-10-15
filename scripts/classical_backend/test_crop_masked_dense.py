"""Exact mask-only common crop and serialized camera golden tests."""

from __future__ import annotations

import json
import importlib.util
import tempfile
import unittest
from unittest.mock import patch
from pathlib import Path
from types import SimpleNamespace

import numpy as np
from PIL import Image

from scripts.classical_backend import crop_masked_dense as crop


SCRATCH = Path(__file__).resolve().parents[2] / ".local-tools" / "tmp"
REAL_MODEL = Path("/Volumes/backups/code/crispmesh-data/mustard-sparse-repair-001/model")
REAL_MASKS = Path("/Volumes/backups/code/crispmesh-data/mustard-sfm-train-001/masks")


class CropTests(unittest.TestCase):
    def setUp(self):
        SCRATCH.mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(dir=SCRATCH)
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def test_asymmetric_union_padding_clip_and_binary_gate(self):
        folder = self.root / "masks"
        folder.mkdir()
        names = ["a.jpg", "b.jpg"]
        for name, box in zip(names, ((0, 13, 33, 44), (1050, 900, 1280, 1024))):
            pixels = np.zeros((1024, 1280), dtype=np.uint8)
            pixels[box[1]:box[3], box[0]:box[2]] = 255
            Image.fromarray(pixels, mode="L").save(folder / (name + ".png"))
        self.assertEqual(crop.common_crop(folder, names), (0, 0, 1280, 1024))
        bad = np.zeros((1024, 1280), dtype=np.uint8)
        bad[500, 500] = 127
        Image.fromarray(bad, mode="L").save(folder / "b.jpg.png")
        with self.assertRaisesRegex(ValueError, "not binary"):
            crop.common_crop(folder, names)

    def test_name_and_principal_shift_only_exact_zero_distortion(self):
        self.assertEqual(crop.png_name("NP3_018.jpg"), "NP3_018.png")
        self.assertEqual(crop.shift_camera("SIMPLE_RADIAL", [1536, 640, 512, 0],
                                            (493, 329, 715, 631)), [1536, 1536, 147, 183])
        with self.assertRaisesRegex(ValueError, "zero-distortion"):
            crop.shift_camera("SIMPLE_RADIAL", [1536, 640, 512, 0.001],
                              (493, 329, 715, 631))
        with self.assertRaises(ValueError):
            crop.png_name("../a.jpg")

    def test_tracked_observation_outside_crop_rejected(self):
        model = SimpleNamespace(
            points3D={1: SimpleNamespace(track=SimpleNamespace(elements=[
                SimpleNamespace(image_id=1, point2D_idx=0)]))},
            images={1: SimpleNamespace(points2D=[SimpleNamespace(xy=np.array([5.5, 6.5]))])})
        with self.assertRaisesRegex(ValueError, "outside"):
            crop.tracked_inside(model, (10, 10, 20, 20))
        self.assertEqual(crop.tracked_inside(model, (0, 0, 10, 10)), 1)

    def test_lossless_cropped_rgb_and_native_mask(self):
        images, masks, output = self.root / "photos", self.root / "source_masks", self.root / "out"
        images.mkdir()
        masks.mkdir()
        output.mkdir()
        name = "NP3_018.jpg"
        rgb = np.zeros((1024, 1280, 3), dtype=np.uint8)
        rgb[200:260, 300:380] = (180, 90, 15)
        Image.fromarray(rgb, mode="RGB").save(images / name, format="JPEG", quality=95)
        label = np.zeros((1024, 1280), dtype=np.uint8)
        label[210:250, 310:370] = 255
        Image.fromarray(label, mode="L").save(masks / (name + ".png"))
        with patch.object(crop.shutil, "disk_usage", return_value=SimpleNamespace(free=30 << 30)):
            record = crop.crop_pixels(images, masks, output, [name],
                                      (290, 190, 390, 270), float("inf"))
        self.assertTrue(record["pixel_equivalent_after_reload"])
        with Image.open(images / name) as original, Image.open(output / "dense/images/NP3_018.png") as saved:
            self.assertTrue(np.array_equal(np.asarray(saved),
                                           np.asarray(original)[190:270, 290:390]))
        with Image.open(output / "masks/NP3_018.mask.png") as saved:
            self.assertTrue(np.array_equal(np.asarray(saved), label[190:270, 290:390]))
        self.assertEqual(json.loads((output / "masks/report.json").read_text())["images"][0]["name"],
                         "NP3_018.png")
        model_dir = output / "dense" / "sparse"
        model_dir.mkdir()
        for filename in crop.MODEL_FILES:
            (model_dir / filename).write_bytes(b"fixture")
        model_record = {"model_sha256": crop.model_hashes(model_dir)}
        self.assertTrue(crop.prepared_unchanged(output, model_record, record))
        (output / "masks/NP3_018.mask.png").write_bytes(b"changed")
        self.assertFalse(crop.prepared_unchanged(output, model_record, record))

    @unittest.skipUnless(importlib.util.find_spec("pycolmap"), "optional native PyCOLMAP fixture")
    def test_synthetic_serialized_crop_preserves_rays_poses_and_tracks(self):
        import pycolmap
        model = pycolmap.Reconstruction()
        camera = pycolmap.Camera(model="SIMPLE_RADIAL", width=1280, height=1024,
                                 params=[1536.0, 640.0, 512.0, 0.0], camera_id=1)
        model.add_camera(camera)
        for image_id, tx, x in ((1, 0.0, 640.0), (2, -0.1, 563.2)):
            image = pycolmap.Image(
                name=f"NP3_{image_id:03d}.jpg",
                keypoints=np.asarray([[x, 512.0], [20.0, 20.0]]),
                cam_from_world=pycolmap.Rigid3d(pycolmap.Rotation3d(),
                                                np.asarray([tx, 0.0, 0.0])),
                camera_id=1, id=image_id)
            model.add_image(image)
            model.register_image(image_id)
        model.add_point3D(np.asarray([0.0, 0.0, 2.0]), pycolmap.Track([
            pycolmap.TrackElement(1, 0), pycolmap.TrackElement(2, 0)]))
        model.check()
        source = self.root / "source"
        source.mkdir()
        model.write_binary(str(source))
        rectangle = (500, 340, 720, 640)
        names = ["NP3_001.jpg", "NP3_002.jpg"]
        record = crop.crop_model(source, self.root / "cropped", rectangle, names, 2)
        self.assertEqual(record["checked_forward_projections_and_rays"], 2)
        self.assertEqual(record["shifted_keypoints"], 4)
        self.assertEqual(record["untracked_keypoints_outside_crop"], 2)
        self.assertLessEqual(record["max_measurement_translation_error_pixels"], 1e-9)
        self.assertLessEqual(record["max_ray_difference"], 1e-9)
        converted = pycolmap.Reconstruction(str(self.root / "cropped"))
        pinhole = next(iter(converted.cameras.values()))
        self.assertEqual(pinhole.model.name, "PINHOLE")
        self.assertEqual((pinhole.width, pinhole.height), (220, 300))
        self.assertEqual(list(pinhole.params), [1536, 1536, 140, 172])
        self.assertEqual(converted.images[1].name, "NP3_001.png")
        self.assertEqual(list(converted.images[1].points2D[1].xy), [-480, -320])
        self.assertEqual(record["model_sha256"]["points3D.bin"],
                         crop.digest(source / "points3D.bin"))

    @unittest.skipUnless(REAL_MODEL.is_dir() and REAL_MASKS.is_dir(),
                         "ignored sealed repair fixture unavailable in CI")
    def test_real_48_camera_binary_serialization_golden(self):
        import pycolmap
        model = pycolmap.Reconstruction(str(REAL_MODEL))
        names = sorted(image.name for image in model.images.values() if image.has_pose)
        self.assertEqual(len(names), 48)
        rectangle = crop.common_crop(REAL_MASKS, names)
        self.assertEqual(rectangle, crop.EXPECTED_CROP)
        self.assertEqual(crop.tracked_inside(model, rectangle), 4734)
        record = crop.crop_model(REAL_MODEL, self.root / "cropped", rectangle, names, 4734)
        self.assertEqual(record["checked_forward_projections_and_rays"], 4734)
        self.assertEqual(record["max_ray_difference"], 0.0)
        self.assertLessEqual(record["max_measurement_translation_error_pixels"], 1e-9)
        self.assertEqual(record["model_sha256"]["points3D.bin"],
                         crop.digest(REAL_MODEL / "points3D.bin"))
        cropped = pycolmap.Reconstruction(str(self.root / "cropped"))
        camera = next(iter(cropped.cameras.values()))
        self.assertEqual((camera.width, camera.height), (222, 302))
        self.assertEqual(camera.model.name, "PINHOLE")
        self.assertEqual(list(camera.params), [1536, 1536, 147, 183])


if __name__ == "__main__":
    unittest.main()
