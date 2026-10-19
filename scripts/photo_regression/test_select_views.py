import unittest
import numpy as np
from scripts.photo_regression.select_views import selection


class ViewSelectionTests(unittest.TestCase):
    def data(self):
        rows = [{'source': str(i), 'rotation': np.eye(3).tolist(), 'translation': [-float(i)/100, 0., -5.]} for i in range(10)]
        audit = {'per_view': [{'photo': str(i), 'inside_fraction': 1-i/20} for i in range(10)]}
        return rows, np.zeros((3, 3)), audit

    def test_ranking_preserves_source_order_and_does_not_change_cameras(self):
        rows, sparse, audit = self.data(); chosen, report = selection(rows, sparse, audit, 8)
        self.assertEqual(chosen, list(range(8)))
        self.assertEqual([v['photo'] for v in report['excluded_sources']], ['8', '9'])
        self.assertAlmostEqual(report['minimum_kept_support'], .65)
        self.assertFalse(report['quality_accepted'])
        self.assertEqual(rows[9]['translation'], [-.09, 0., -5.])

    def test_excluding_a_distinct_direction_is_refused(self):
        rows, sparse, audit = self.data(); rows[9]['translation'] = [0., 0., 5.]
        with self.assertRaises(ValueError): selection(rows, sparse, audit, 8)

    def test_bad_names_support_and_rigid_poses_are_refused(self):
        for mode in ['duplicate', 'nan', 'mapping', 'rotation']:
            rows, sparse, audit = self.data(); mapping = None
            if mode == 'duplicate': rows[1]['source'] = rows[0]['source']
            if mode == 'nan': audit['per_view'][0]['inside_fraction'] = float('nan')
            if mode == 'mapping': mapping = {'photos': [{'capture': '0', 'source': 'x'}]}
            if mode == 'rotation': rows[0]['rotation'][0][0] = 2.
            with self.subTest(mode=mode), self.assertRaises(ValueError): selection(rows, sparse, audit, 8, mapping)


if __name__ == '__main__': unittest.main()
