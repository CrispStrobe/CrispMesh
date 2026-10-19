import unittest
import numpy as np
from scripts.photo_regression.validate_masks import dark_hole_fraction, validate_refinement


class MaskValidationTests(unittest.TestCase):
    def test_native_dark_hole_policy_ignores_open_channels(self):
        rgb = np.full((60, 60, 3), 80, 'uint8'); mask = np.zeros((60, 60), bool); mask[5:55, 5:55] = True
        mask[10:15, 10:15] = False; mask[40:55, 28:32] = False
        self.assertAlmostEqual(dark_hole_fraction(rgb, mask), 25/mask.sum())
        rgb[10:15, 10:15] = 131
        self.assertEqual(dark_hole_fraction(rgb, mask), 0.)

    def test_false_holes_through_flat_foreground_revert_to_coarse(self):
        prior = np.zeros((100, 100), bool); prior[10:90, 20:80] = True
        rgb = np.full((100, 100, 3), 190, 'uint8'); rgb[prior] = [40, 80, 180]
        candidate = prior.copy(); candidate[30:50, 35:55] = False
        result, report = validate_refinement(rgb, prior, candidate)
        self.assertTrue(report['coarse_fallback'])
        np.testing.assert_array_equal(result, prior)

    def test_empty_refined_candidate_keeps_a_valid_coarse_fallback(self):
        prior = np.zeros((30, 30), bool); prior[5:25, 5:25] = True
        rgb = np.full((30, 30, 3), 180, 'uint8')
        result, report = validate_refinement(rgb, prior, np.zeros_like(prior))
        self.assertTrue(report['coarse_fallback'])
        self.assertIn('empty_refined_mask', report['reasons'])
        np.testing.assert_array_equal(result, prior)

    def test_visible_background_leg_gap_is_preserved(self):
        prior = np.zeros((100, 100), bool); prior[10:90, 20:80] = True
        rgb = np.full((100, 100, 3), 190, 'uint8'); rgb[prior] = [10, 20, 30]
        candidate = prior.copy(); candidate[60:90, 46:54] = False; rgb[60:90, 46:54] = 190
        result, report = validate_refinement(rgb, prior, candidate)
        self.assertFalse(report['coarse_fallback'])
        np.testing.assert_array_equal(result, candidate)


if __name__ == '__main__': unittest.main()
