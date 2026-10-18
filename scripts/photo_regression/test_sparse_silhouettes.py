import unittest

import numpy as np
from scripts.photo_regression.sparse_silhouettes import projection_support, validate_dimensions


class SparseSilhouetteTests(unittest.TestCase):
    @staticmethod
    def project(points):
        return points[:, :2] / points[:, 2, None] + .5

    def test_first_last_pixels_and_excluded_mask(self):
        mask = np.ones((3, 4), bool)
        mask[1, 2] = False
        points = [[0, 0, 1], [3, 2, 1], [2, 1, 1]]
        result = projection_support(points, lambda p: p, self.project, mask)
        self.assertAlmostEqual(result['inside_fraction'], 2 / 3)

    def test_off_frame_behind_and_nonfinite_never_add_support(self):
        points = [[0, 0, 1], [5, 0, 1], [0, 0, -1], [1e30, 0, 1]]
        result = projection_support(points, lambda p: p, self.project, np.ones((2, 2), bool))
        self.assertEqual(result['inside_fraction'], .25)
        self.assertEqual(result['behind_or_nonfinite_camera_fraction'], .25)
        def unexpected_projection(_):
            self.fail('behind-camera points must not reach projection')
        result = projection_support([[0, 0, -1]], lambda p: p, unexpected_projection, np.ones((2, 2), bool))
        self.assertEqual(result['inside_fraction'], 0.)

    def test_diagnostic_scaling_keeps_corner_convention(self):
        mask = np.zeros((8, 8), bool)
        mask[2, 2] = True
        result = projection_support([[0, 0, 1]], lambda p: p, self.project, mask, pixel_scale=4)
        self.assertEqual(result['inside_fraction'], 1.)
        validate_dimensions(mask.shape, 2, 2, 4)
        with self.assertRaises(ValueError):
            validate_dimensions(mask.shape, 4, 4, 4)

    def test_nonfinite_and_empty_inputs_are_refused(self):
        with self.assertRaises(ValueError):
            projection_support([], lambda p: p, self.project, np.ones((2, 2)))
        with self.assertRaises(ValueError):
            projection_support([[np.nan, 0, 1]], lambda p: p, self.project, np.ones((2, 2)))
        result = projection_support([[0, 0, 1]], lambda p: p, lambda p: np.array([[np.nan, 0]]), np.ones((2, 2)))
        self.assertEqual(result['inside_fraction'], 0)
        self.assertEqual(result['nonfinite_projection_count'], 1)


if __name__ == '__main__':
    unittest.main()
