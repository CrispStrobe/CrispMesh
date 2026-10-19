import unittest
import numpy as np
from scripts.photo_regression.learned_matches import candidate_graph, source_keypoints, square_crop, validated_pair


class LearnedMatchTests(unittest.TestCase):
    def test_crop_never_clips_foreground_even_at_frame_edge(self):
        m = np.zeros((9, 7), 'uint8');m[:8, 4:] = 255
        l, t, r, b = square_crop(m)
        self.assertEqual(r - l, b - t)
        self.assertLessEqual(l, 4);self.assertLessEqual(t, 0)
        self.assertGreaterEqual(r, 7);self.assertGreaterEqual(b, 8)
        with self.assertRaises(ValueError):
            square_crop(np.zeros((3, 4), 'uint8'))

    def test_resize_inverse_preserves_offcentre_half_pixel(self):
        points = np.array([[0., 0.], [127., 127.]])
        original = source_keypoints(points, [-13, 4, 243, 260], 128)
        np.testing.assert_allclose(original, [[-12.5, 4.5], [241.5, 258.5]])
        np.testing.assert_allclose((original + .5 - [-13, 4]) / 2 - .5, points)

    def test_graph_is_bounded_deterministic_and_has_no_duplicate_votes(self):
        descriptors = np.random.default_rng(42).normal(size=(117, 16))
        pairs = candidate_graph(descriptors, neighbors=6, temporal_radius=2)
        self.assertEqual(pairs, candidate_graph(descriptors, 6, 2))
        self.assertLessEqual(len(pairs), 117 * (6 + 4))
        self.assertEqual(len(pairs), len(set(pairs)))
        self.assertTrue(all(a < b for a, b in pairs))
        self.assertIn((0, 2), pairs)

    def test_retrieval_can_link_out_of_order_views(self):
        d = np.eye(4);d[3] = d[0]
        self.assertIn((0, 3), candidate_graph(d, 1, 0))
        self.assertEqual(candidate_graph(d, 0, 0), [])
        self.assertEqual(candidate_graph(np.zeros((3, 2)), 1, 0), [(0, 1), (0, 2)])

    def test_pair_cache_rejects_out_of_range_and_duplicate_votes(self):
        self.assertEqual(validated_pair([], 2, 3).shape, (0, 2))
        np.testing.assert_array_equal(validated_pair([[0, 2], [1, 0]], 2, 3), [[0, 2], [1, 0]])
        for matches in [[[0, 3]], [[-1, 0]], [[0., 1.]], [[0, 0], [0, 1]], [[0, 0], [1, 0]]]:
            with self.assertRaises(ValueError):
                validated_pair(matches, 2, 3)

    def test_bad_descriptors_and_budget_refused(self):
        for d in [np.zeros(4), np.array([[np.nan]])]:
            with self.assertRaises(ValueError):
                candidate_graph(d)
        with self.assertRaises(ValueError):
            candidate_graph(np.zeros((2, 2)), -1)


if __name__ == '__main__':
    unittest.main()
