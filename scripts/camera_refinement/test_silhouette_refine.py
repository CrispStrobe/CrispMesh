import tempfile
from pathlib import Path
import unittest
import numpy as np
from PIL import Image
from scripts.camera_refinement.silhouette_refine import moved,project,boundary_points,refine

class CameraRefinement(unittest.TestCase):
    def setUp(self):
        self.triangles=np.array([[[-1,-1,0],[1,-1,0],[1,1,0]],[[-1,-1,0],[1,1,0],[-1,1,0]]],float)
        self.row={'rotation':np.eye(3).tolist(),'translation':[0.,0.,5.],'width':128,'height':128,'k':[100.,100.,64.,64.]}
    def test_identity_update_and_rendered_contour_share_projection(self):
        rotation,translation=moved(self.row,np.zeros(6),np.zeros(3),2.)
        np.testing.assert_allclose(rotation,self.row['rotation']);np.testing.assert_allclose(translation,self.row['translation'])
        points,mask=boundary_points(self.triangles,self.row,128,128,np.array(self.row['k']))
        xy=project(points,rotation,translation,np.array(self.row['k']))
        np.testing.assert_allclose(xy,np.round(xy),atol=1e-8)
        self.assertTrue(mask[np.round(xy[:,1]).astype(int),np.round(xy[:,0]).astype(int)].all())
    def test_known_pose_error_improves_with_independent_observation_check(self):
        _,mask=boundary_points(self.triangles,self.row,128,128,np.array(self.row['k']))
        y,x=np.mgrid[-.8:.81:10j,-.8:.81:10j];points=np.column_stack([x.ravel(),y.ravel(),np.zeros(x.size)])
        observed=project(points,np.eye(3),np.array([0.,0.,5.]),np.array(self.row['k']))
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/'own-synthetic-mask.png';Image.fromarray(mask.astype('uint8')*255).save(path)
            wrong=dict(self.row,translation=[.06,-.04,5.],_mask=str(path))
            updated,report=refine(wrong,self.triangles,points,observed,np.zeros(3),2.)
        self.assertTrue(report['accepted'],report)
        self.assertGreaterEqual(report['candidate_mask_iou'],report['initial_mask_iou'])
        self.assertLess(report['after']['feature_heldout_median_px'],report['before']['feature_heldout_median_px']*.5)
        self.assertFalse(np.allclose(updated['translation'],wrong['translation']))
