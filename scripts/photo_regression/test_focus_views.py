import unittest
import cv2
import numpy as np
from scripts.photo_regression.focus_views import sharpness,select

class FocusViews(unittest.TestCase):
    def test_object_blur_reduces_sharpness_and_flat_object_is_not_an_error(self):
        y,x=np.mgrid[:512,:512];gray=(80+90*(x>256)+40*np.sin(x*.25)*np.sin(y*.17)).astype('uint8')
        rgb=np.repeat(gray[:,:,None],3,axis=2);mask=np.zeros((512,512),bool);mask[30:480,30:480]=True
        self.assertLess(sharpness(cv2.GaussianBlur(rgb,(0,0),4),mask),sharpness(rgb,mask)*.6)
        self.assertEqual(sharpness(np.full_like(rgb,127),mask),0)
    def test_soft_unique_direction_and_minimum_views_are_retained(self):
        directions=np.array([[0,0,1]]*8+[[1,0,0]],float)
        kept,report=select([1]*8+[.01],directions)
        self.assertEqual(kept,list(range(9)))
        # A nearby redundant soft photo can be excluded without equal angle assumptions.
        directions=np.array([[0,0,1]]*9,float)
        kept,report=select([1]*8+[.01],directions)
        self.assertEqual(kept,list(range(8)))
        kept,_=select([1]*7+[.01],directions[:8])
        self.assertEqual(len(kept),8)
    def test_deleting_a_chain_cannot_break_coverage_of_an_earlier_exclusion(self):
        angles=np.radians([-19,0,0]+[19]*8)
        directions=np.column_stack([np.sin(angles),np.zeros(11),np.cos(angles)])
        kept,report=select([.02,.2,.2]+[1]*8,directions)
        self.assertNotIn(0,kept)
        self.assertTrue(set(kept)&{1,2})
        self.assertTrue(set(report['coverage_protected_indices'])&{1,2})

    def test_missing_or_textureless_scores_do_not_trigger_bulk_deletion(self):
        kept,_=select([None]+[0]*8,[[0,0,1]]*9)
        self.assertEqual(len(kept),9)
        with self.assertRaises(ValueError):select([1]*9,[[0,0,0]]*9)
