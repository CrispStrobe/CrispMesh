import unittest
import numpy as np
from scripts.photo_regression.colmap_inputs import native_intrinsics, object_crop


class ColmapInputTests(unittest.TestCase):
    def test_corner_convention_round_trip_through_offcentre_crop(self):
        # A measured point at the first retained pixel's centre must stay at .5.
        k = native_intrinsics('PINHOLE', [90, 110, 6, 4], crop_origin=(2, 1))
        point = np.array([(2.5 - 6) / 90, (1.5 - 4) / 110, 1.])
        projected = point[:2] / point[2] * k[:2] + k[2:]
        np.testing.assert_allclose(projected, [.5, .5], atol=1e-12)
        np.testing.assert_allclose(np.array(k[2:]) - .5, [3.5, 2.5])

    def test_scaled_diagnostic_camera_retains_original_pixel_projection(self):
        k = native_intrinsics('FULL_OPENCV', [750, 750, 378, 504] + [0] * 8, 4, (1510, 2000))
        self.assertEqual(k, [3000., 3000., 2., 16.])
        q = np.array([.1, -.05, 2.])
        expected = (q[:2] / q[2] * 750 + [378, 504]) * 4 - [1510, 2000]
        np.testing.assert_allclose(q[:2] / q[2] * k[:2] + k[2:], expected)

    def test_equivalent_models_match_production_half_centre_intrinsics(self):
        # Production scene_writer exports a Lens cx=31.5,cy=23.5 as K cx=32,cy=24.
        for model, p in [('SIMPLE_PINHOLE', [80, 32, 24]), ('SIMPLE_RADIAL', [80, 32, 24, 0]),
                         ('PINHOLE', [80, 80, 32, 24]), ('FULL_OPENCV', [80, 80, 32, 24] + [0] * 8)]:
            self.assertEqual(native_intrinsics(model, p), [80., 80., 32., 24.])

    def test_distortion_and_invalid_calibration_are_refused(self):
        for model, p in [('SIMPLE_RADIAL', [90, 6, 4, .1]), ('PINHOLE', [-90, 90, 6, 4]), ('PINHOLE', [90, 90, np.nan, 4])]:
            with self.assertRaises(ValueError):
                native_intrinsics(model, p)
        for scale in [0, -1, np.inf]:
            with self.assertRaises(ValueError):
                native_intrinsics('SIMPLE_PINHOLE', [90, 6, 4], scale)

    def test_foreground_crop_preserves_edges_and_checks_empty_mask(self):
        mask = np.zeros((8, 10), 'uint8');mask[0:4, 5:10] = 255
        self.assertEqual(object_crop(mask, .2), [4, 0, 10, 5])
        with self.assertRaises(ValueError):
            object_crop(np.zeros((4, 4), 'uint8'), .15)


if __name__ == '__main__':
    unittest.main()
