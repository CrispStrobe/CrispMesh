import unittest
import numpy as np
from scripts.photo_regression.refine_masks import foreground_cues, clean_candidate, choose_candidate


class MaskRefinementTests(unittest.TestCase):
    def test_partial_high_score_cannot_win_with_only_a_torso_cue(self):
        coarse = np.zeros((80, 80), bool); coarse[10:70, 20:60] = True
        full = coarse.copy(); full[55:70, 38:42] = False
        part = coarse.copy(); part[40:] = False
        result, selected, rows = choose_candidate([part, full], [.99, .7], coarse, [[30, 20], [0, 0]], [1, 0])
        self.assertEqual(selected, 1)
        self.assertFalse(rows[0]['eligible'])
        self.assertFalse(result[65, 40])
        self.assertTrue(result[65, 30])

    def test_failed_cues_and_excessive_cleanup_keep_coarse_mask(self):
        coarse = np.zeros((40, 40), bool); coarse[5:35, 5:35] = True
        candidates = np.stack([coarse.copy(), coarse.copy()]); candidates[:, 20, 20] = False
        result, selected, _ = choose_candidate(candidates, [.9, .8], coarse, [[10, 10]], [1], maximum_cleanup=0)
        self.assertIsNone(selected)
        np.testing.assert_array_equal(result, coarse)
        with self.assertRaises(ValueError):
            choose_candidate(candidates, [.9, .8], coarse, [[0, 0]], [0])

    def test_large_openings_survive_small_noise_cleanup(self):
        mask = np.zeros((80, 80), bool); mask[10:70, 10:70] = True
        mask[55:70, 38:42] = False; mask[25:30, 25:30] = False
        mask[20, 20] = False; mask[3, 3] = True
        clean = clean_candidate(mask)
        self.assertTrue(clean[20, 20])
        self.assertFalse(clean[3, 3])
        self.assertFalse(clean[27, 27])
        self.assertFalse(clean[65, 40])

    def test_cues_avoid_background_coloured_gap_inside_coarse_prior(self):
        coarse = np.zeros((90, 90), bool); coarse[10:80, 20:70] = True
        rgb = np.full((90, 90, 3), 180, 'uint8'); rgb[coarse] = [20, 40, 130]
        rgb[55:80, 43:47] = 180
        coords, labels = foreground_cues(rgb, coarse)
        self.assertGreater(sum(labels), 1)
        for (x, y), label in zip(coords, labels):
            if label: self.assertFalse(np.array_equal(rgb[y, x], [180, 180, 180]))
        with self.assertRaises(ValueError):
            foreground_cues(rgb, np.ones_like(coarse))


if __name__ == '__main__':
    unittest.main()
