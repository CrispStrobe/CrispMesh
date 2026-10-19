import unittest
import numpy as np
from scripts.photo_regression.filter_matches import foreground_flags, filter_pair


class ForegroundMatchTests(unittest.TestCase):
    def test_margin_excludes_boundary_without_changing_source_indices(self):
        mask = np.zeros((20, 20), bool); mask[2:18, 2:18] = True
        points = [[10., 10.], [2., 3.], [1.49, 10.], [1e200, 0.], [-1e200, 0.]]
        np.testing.assert_array_equal(foreground_flags(points, mask, 3), [True, False, False, False, False])
        first = np.array([True, False, True]); second = np.array([False, True, True])
        self.assertEqual(filter_pair([[0, 1], [1, 2], [2, 0]], first, second), [[0, 1]])

    def test_nearest_pixel_centres_and_bad_measurements(self):
        mask = np.zeros((8, 8), bool); mask[3, 4] = True
        np.testing.assert_array_equal(foreground_flags([[3.5, 3.], [3.49, 3.]], mask), [True, False])
        with self.assertRaises(ValueError): foreground_flags([[float('nan'), 0]], mask)
        with self.assertRaises(ValueError): filter_pair([[0, 0], [0, 1]], np.ones(2, bool), np.ones(2, bool))


if __name__ == '__main__': unittest.main()
