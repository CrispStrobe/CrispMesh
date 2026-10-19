import unittest
import numpy as np
from scripts.photo_regression.prepare_capture import choose_instance, common_crop, initial_lens
from scripts.photo_regression.capture_pipeline import selected_count, camera_acceptable, camera_search_order, geometry_acceptable

class CapturePreparation(unittest.TestCase):
    def test_camera_preference_retains_all_alternative_paths(self):
        for preferred in ['hybrid','tight','wide']:
            order=camera_search_order(preferred)
            self.assertEqual(order[0],preferred)
            self.assertEqual(set(order),{'hybrid','tight','wide'})
            self.assertEqual(len(order),3)
        with self.assertRaises(ValueError):camera_search_order('unknown')
    def test_registration_count_does_not_approve_inconsistent_real_capture(self):
        self.assertFalse(camera_acceptable({'models':[{'cameras':67}]},{'median_inside_fraction':.779},76))
        self.assertFalse(camera_acceptable({'models':[{'cameras':38}]},{'median_inside_fraction':.93},76))
        self.assertTrue(camera_acceptable({'models':[{'cameras':108}]},{'median_inside_fraction':.909},116))
        self.assertFalse(camera_acceptable({'models':[{'cameras':108}]},{'median_inside_fraction':float('nan')},116))
    def test_sparse_support_does_not_override_a_bad_actual_silhouette(self):
        self.assertTrue(camera_acceptable({'models':[{'cameras':117}]},{'median_inside_fraction':.8784},117))
        self.assertFalse(geometry_acceptable(.773494))
        self.assertTrue(geometry_acceptable(.897456))
        for value in [None,False,float('nan'),float('inf'),-.1,1.1]:
            self.assertFalse(geometry_acceptable(value))

    def test_broad_support_is_not_selected_over_a_bounded_object(self):
        paper=np.ones((100,100),np.uint8)*255
        object_mask=np.zeros_like(paper);object_mask[30:70,40:60]=255
        self.assertEqual(choose_instance([('paper',paper),('object',object_mask)])[1],'object')
        self.assertIsNone(choose_instance([('paper',paper)]))

    def test_shared_crop_preserves_offcentre_projection_coordinates(self):
        crop=common_crop([[20,30,80,90],[30,10,90,70]],100,100)
        lens=initial_lens(100,100,crop)
        xyz=np.array([.1,-.2,1.])
        projected=.9*100*xyz[:2]/xyz[2]+49.5
        np.testing.assert_allclose(np.array(lens['pixels'][2:])+lens['pixels'][0]*xyz[:2]/xyz[2],projected-np.array(crop[:2]))

    def test_count_policy_is_fractional_and_has_no_fixed_angle_or_named_views(self):
        self.assertEqual(selected_count(116),106)
        self.assertEqual(selected_count(76),69)
        self.assertEqual(selected_count(8),8)
        with self.assertRaises(ValueError):selected_count(7)
