import unittest

import numpy as np

from scripts.photo_regression.sift_overlap import mutual_pairs, strongest_edges


class OverlapTests(unittest.TestCase):
    def test_mutual_matching_does_not_duplicate_a_feature(self):
        first = np.eye(4, dtype='float32')
        second = first[[2, 0, 3, 1]]
        self.assertEqual(mutual_pairs(first, second), [[0, 1], [1, 3], [2, 0], [3, 2]])
        self.assertEqual(mutual_pairs(first[:1], second), [])
        ambiguous = np.ones((3, 4), dtype='float32')
        self.assertEqual(mutual_pairs(ambiguous, ambiguous), [])

    def test_overlap_proposals_are_bounded_and_keep_strong_edges(self):
        count = 40
        pairs = [{'first': a, 'second': b, 'matches': [[0, 0]] * (12 + a + b)}
                 for a in range(count) for b in range(a + 1, count)]
        selected = strongest_edges(pairs, count)
        self.assertLessEqual(len(selected), count * 4)
        self.assertIn(pairs[-1], selected)
        self.assertEqual(selected, strongest_edges(pairs, count))
        self.assertEqual(strongest_edges([{'first': 0, 'second': 1, 'matches': []}], 2), [])
