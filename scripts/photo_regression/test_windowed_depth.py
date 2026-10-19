import unittest
import numpy as np
from scipy.spatial.transform import Rotation
from scripts.photo_regression.windowed_depth import virtual_camera, align_points, anchor_depth, reframe_depth, window_for, original_sources


class WindowedDepthTests(unittest.TestCase):
    def test_virtual_rotation_preserves_offcentre_pinhole_rays(self):
        mask = np.zeros((120, 160), 'uint8');mask[4:45, 112:156] = 255
        half_k = [190., 210., 80., 60.]
        h, q, target = virtual_camera(mask, half_k, 112)
        source = np.array([[190., 0, 79.5], [0, 210., 59.5], [0, 0, 1.]])
        rays = np.random.default_rng(42).normal(size=(3, 40));rays[2] = 4.
        a = h @ source @ rays;b = target @ q @ rays
        np.testing.assert_allclose(a[:2] / a[2], b[:2] / b[2], atol=1e-12)
        np.testing.assert_allclose(q @ q.T, np.eye(3), atol=1e-14)
        self.assertAlmostEqual(np.linalg.det(q), 1.)
        with self.assertRaises(ValueError):
            virtual_camera(np.zeros((4, 4)), half_k, 112)

    def test_own_landmark_world_similarity_keeps_camera_geometry(self):
        centres = np.array([[1., 0, 3], [-1., 0, 3], [0, 2., 3], [0, 0, -2.]])
        r = Rotation.from_rotvec([.2, -.3, .4]).as_matrix();offset = np.array([3., 1., -4.]);scale = 2.3
        poses, rows = {}, []
        for i, c in enumerate(centres):
            name = str(i);poses[name] = np.eye(3), -c
            target_c = scale * r @ c + offset;target_r = r.T
            rows.append({'source': name, 'rotation': target_r.tolist(), 'translation': (-target_r @ target_c).tolist()})
        points = np.array([[.1, .2, .3], [2., 0, -1.]])
        np.testing.assert_allclose(align_points(poses, rows, points), scale * points @ r.T + offset, atol=1e-12)
        rows[0]['rotation'] = (Rotation.from_rotvec([.1, 0, 0]).as_matrix() @ r.T).tolist()
        with self.assertRaises(ValueError):
            align_points(poses, rows, points)

    def test_sparse_scale_uses_observed_geometry_and_rejects_outliers(self):
        depth = np.full((20, 24), 2., dtype='float32');mask = np.ones_like(depth, dtype=bool)
        k = np.array([[20., 0, 12.], [0, 20., 10.], [0, 0, 1.]])
        u = np.linspace(3, 20, 60);v = np.linspace(3, 16, 60);z = np.full(60, 4.);z[::4] = 40.
        points = np.column_stack(((u - 12) / 20 * z, (v - 10) / 20 * z, z))
        result, report = anchor_depth(depth, mask, np.eye(3), np.zeros(3), k, points)
        np.testing.assert_array_equal(result, 4.)
        self.assertEqual(report['scale'], 2.)
        self.assertLess(report['retained_fit_points'], report['fit_points'])
        self.assertGreater(report['heldout_points'], 0)

    def test_insufficient_scale_support_returns_no_depth(self):
        depth = np.ones((20, 24), 'float32');k = np.array([[20., 0, 12.], [0, 20., 10.], [0, 0, 1.]])
        result, report = anchor_depth(depth, depth > 0, np.eye(3), np.zeros(3), k, np.zeros((3, 3)))
        self.assertIsNone(result)
        self.assertEqual(report['status'], 'insufficient_observed_scale_landmarks')

    def test_level_reframing_keeps_plane_z_without_half_pixel_shift(self):
        depth = np.full((10, 14), 2., 'float32')
        result = reframe_depth(depth, [20., 23., 7., 4.], [10., 11.5, 2.5, 1.5], 5, 3)
        np.testing.assert_array_equal(result, np.full((3, 5), 2., 'float32'))
        depth[3:5, 6:8] = 0
        result = reframe_depth(depth, [20., 23., 7., 4.], [20., 23., 7., 4.], 14, 10)
        self.assertEqual(result[3, 6], 0)
        self.assertTrue(np.isfinite(result).all())

    def test_staged_photo_names_require_explicit_one_to_one_mapping(self):
        rows = [{'source': 'capture_001.png', 'name': 'a'}, {'source': 'capture_000.png', 'name': 'b'}]
        mapping = {'photos': [{'capture': 'capture_000.png', 'source': 'first.jpg'},
                              {'capture': 'capture_001.png', 'source': 'second.jpg'}]}
        resolved = original_sources(rows, mapping)
        self.assertEqual([row['source'] for row in resolved], ['second.jpg', 'first.jpg'])
        self.assertEqual(resolved[0]['input_source'], 'capture_001.png')
        self.assertEqual(rows[0]['source'], 'capture_001.png')
        with self.assertRaises(ValueError):
            original_sources(rows, {'photos': mapping['photos'][:1]})
        with self.assertRaises(ValueError):
            original_sources(rows, {'photos': mapping['photos'] * 2})
        with self.assertRaises(ValueError):
            original_sources(rows * 2)

    def test_duplicate_camera_directions_do_not_form_a_fake_window(self):
        with self.assertRaises(ValueError):
            window_for(0, {0, 1, 2}, np.array([[0., 0, 1.]] * 3), 4)
        angles = np.radians([0, 10, 20, 35, 70])
        directions = np.column_stack((np.sin(angles), np.zeros(5), np.cos(angles)))
        group = window_for(0, set(range(5)), directions, 4)
        self.assertEqual(group[0], 0)
        self.assertEqual(len(group), len(set(group)))
        self.assertNotIn(4, group)


if __name__ == '__main__':
    unittest.main()
