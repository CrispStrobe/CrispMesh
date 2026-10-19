import copy
import unittest
from types import SimpleNamespace
import numpy as np
from scripts.photo_regression.colmap_recover import validate_cache, prune_support


class ColmapCacheTests(unittest.TestCase):
    def test_support_pruning_rechecks_tracks_after_removing_a_weak_pose(self):
        class Model:
            def __init__(self):
                self.images={i:SimpleNamespace(name=f'photo-{i}.jpg',num_points3D=n,pose=(i,2,3)) for i,n in [(1,16),(2,20),(3,60)]}
            def reg_image_ids(self):return list(self.images)
            def deregister_image(self,index):
                del self.images[index]
                if index==1:self.images[2].num_points3D-=1
        model=Model();original_pose=model.images[3].pose
        removed=prune_support(model,20)
        self.assertEqual(removed,[{'name':'photo-1.jpg','observations':16},{'name':'photo-2.jpg','observations':19}])
        self.assertEqual(list(model.images),[3]);self.assertEqual(model.images[3].pose,original_pose)
        unchanged=Model();self.assertEqual(prune_support(unchanged,0),[]);self.assertEqual(len(unchanged.images),3)
        with self.assertRaises(ValueError):prune_support(unchanged,-1)

    def cache(self):
        return {'names': ['a.jpg', 'b.jpg', 'c.jpg'], 'keypoints': [[[0., 0.], [3., 4.]]] * 3,
                'lens': {'width': 20, 'height': 10, 'pixels': [30., 30., 9.5, 4.5], 'k': [.01, 0., 0.]},
                'pairs': [{'first': 0, 'second': 1, 'matches': [[0, 1], [1, 0]]}]}

    def test_source_to_colmap_pixel_centres_preserve_projection(self):
        params, keys, pairs = validate_cache(self.cache())
        self.assertEqual(params, [30., 10., 5., .01])
        np.testing.assert_array_equal(keys[0], [[.5, .5], [3.5, 4.5]])
        np.testing.assert_array_equal((keys[0] - params[1:3]) / params[0],
                                      (np.array(self.cache()['keypoints'][0]) - [9.5, 4.5]) / 30.)
        self.assertEqual(pairs[0][2].dtype, np.dtype('uint32'))

    def test_invalid_lens_pairs_and_nonfinite_measurements_refused(self):
        for change in ['lens', 'duplicate_pair', 'ambiguous_pair', 'nonfinite', 'path']:
            data = copy.deepcopy(self.cache())
            if change == 'lens': data['lens']['k'][2] = .1
            if change == 'duplicate_pair': data['pairs'] *= 2
            if change == 'ambiguous_pair': data['pairs'][0]['matches'] = [[0, 0], [0, 1]]
            if change == 'nonfinite': data['keypoints'][0][0][0] = float('nan')
            if change == 'path': data['names'][0] = '../a.jpg'
            with self.subTest(change=change), self.assertRaises(ValueError):
                validate_cache(data)


if __name__ == '__main__':
    unittest.main()
