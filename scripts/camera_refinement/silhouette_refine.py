#!/usr/bin/env python
"""Experimental second-pass own-hull camera refinement; original path preserved.

Original implementation using our CPU mesh renderer and SciPy least squares.
No upstream algorithm source, reference mesh or supplied camera is consumed.
Refines bounded extrinsics with contour distances AND actual own SfM observations.
Focal/distortion and sparse 3D structure stay fixed. Not joint bundle adjustment.
"""
import argparse
import json
import os
from pathlib import Path
import sys
import time

ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT))
import cv2
import numpy as np
from PIL import Image
from scipy.ndimage import map_coordinates
from scipy.optimize import least_squares
from scipy.spatial import cKDTree
from scipy.spatial.transform import Rotation
from scripts.turntable_mesh.stl_compare_render import read_stl,rasterise


def project(points,rotation,translation,k):
    camera=points@rotation.T+translation
    return camera[:,:2]/camera[:,2,None]*k[:2]+k[2:]


def moved(row,delta,centre,radius):
    rotation=np.asarray(row['rotation']);translation=np.asarray(row['translation'])
    update=Rotation.from_rotvec(delta[:3]).as_matrix();pivot=rotation@centre+translation
    return update@rotation,update@(translation-pivot)+pivot+delta[3:]*radius


def boundary_points(triangles,row,width,height,k,limit=256):
    rotation=np.asarray(row['rotation']);translation=np.asarray(row['translation'])
    camera=triangles@rotation.T+translation
    xy=camera[:,:,:2]/camera[:,:,2:]*k[:2]+k[2:]
    valid=(camera[:,:,2]>1e-8).all(1)&(xy.max(1)>=0).all(1)&(xy.min(1)<[width,height]).all(1)
    ids=np.flatnonzero(valid)
    raster=rasterise(xy[valid],camera[valid,:,2],ids.astype(float),width,height)
    mask=np.isfinite(raster);edge=mask&~cv2.erode(mask.astype('uint8'),np.ones((3,3),'uint8')).astype(bool)
    y,x=np.where(edge)
    if len(x)<16:raise ValueError('too little visible hull boundary')
    chosen=np.linspace(0,len(x)-1,min(limit,len(x))).astype(int);x=x[chosen];y=y[chosen]
    faces=triangles[raster[y,x].astype(int)]
    normals=np.cross(faces[:,1]-faces[:,0],faces[:,2]-faces[:,0])
    origin=-rotation.T@translation
    rays=np.column_stack([(x-k[2])/k[0],(y-k[3])/k[1],np.ones(len(x))])@rotation
    denominator=np.sum(normals*rays,axis=1)
    ok=np.abs(denominator)>1e-12
    points=origin+rays[ok]*(np.sum(normals[ok]*(faces[ok,0]-origin),axis=1)/denominator[ok])[:,None]
    return points,mask


def refine(row,triangles,points,observations,centre,radius,max_evaluations=40,feature_weight=10.):
    mask=np.array(Image.open(row['_mask']).convert('L'))>127
    scale=min(1.,256/max(mask.shape));height,width=[max(8,round(value*scale)) for value in mask.shape]
    mask=cv2.resize(mask.astype('uint8'),(width,height),interpolation=cv2.INTER_NEAREST)>0
    factors=np.array([width/row['width'],height/row['height']]);k=np.asarray(row['k'])*np.tile(factors,2)
    edge=mask&~cv2.erode(mask.astype('uint8'),np.ones((3,3),'uint8')).astype(bool)
    y,x=np.where(edge)
    if len(x)<16 or len(points)<20:raise ValueError('insufficient object contour or own feature observations')
    observed_boundary=np.column_stack([x,y]);chosen=np.linspace(0,len(x)-1,min(256,len(x))).astype(int)
    observed_boundary=observed_boundary[chosen]
    distance=cv2.distanceTransform((~edge).astype('uint8'),cv2.DIST_L2,cv2.DIST_MASK_PRECISE).astype('float64')
    contour,initial_mask=boundary_points(triangles,row,width,height,k)
    observed=observations*factors
    train=np.arange(len(points))%5!=0;held=~train
    r0=np.asarray(row['rotation']);t0=np.asarray(row['translation'])
    def metrics(rotation,translation):
        uv=project(contour,rotation,translation,k)
        contour_error=map_coordinates(distance,[uv[:,1],uv[:,0]],order=1,mode='constant',cval=256.)
        features=np.linalg.norm(project(points,rotation,translation,k)-observed,axis=1)
        return {'contour_mean_px':float(np.mean(contour_error)),'feature_fit_median_px':float(np.median(features[train])),
            'feature_heldout_median_px':float(np.median(features[held])),
            'feature_heldout_p95_px':float(np.percentile(features[held],95))}
    def residual(delta):
        rotation,translation=moved(row,delta,centre,radius)
        uv=project(contour,rotation,translation,k)
        forward=map_coordinates(distance,[uv[:,1],uv[:,0]],order=1,mode='constant',cval=256.)
        backward=cKDTree(uv).query(observed_boundary)[0]
        features=(project(points[train],rotation,translation,k)-observed[train]).reshape(-1)
        # Normalize terms by sample count; retain feature geometry and small pose updates.
        return np.concatenate([forward/np.sqrt(len(forward)),backward/np.sqrt(len(backward)),
            feature_weight*features/np.sqrt(len(features)),.2*delta[:3]/np.radians(1),.2*delta[3:]/.01])
    bound=np.array([np.radians(5)]*3+[.05]*3)
    before=metrics(r0,t0)
    result=least_squares(residual,np.zeros(6),bounds=(-bound,bound),diff_step=1e-4,
        loss='soft_l1',f_scale=1.,max_nfev=max_evaluations)
    rotation,translation=moved(row,result.x,centre,radius);after=metrics(rotation,translation)
    candidate_row=dict(row,rotation=rotation.tolist(),translation=translation.tolist())
    _,candidate_mask=boundary_points(triangles,candidate_row,width,height,k)
    initial_iou=float((initial_mask&mask).sum()/max(1,(initial_mask|mask).sum()))
    candidate_iou=float((candidate_mask&mask).sum()/max(1,(candidate_mask|mask).sum()))
    accepted=(after['contour_mean_px']<before['contour_mean_px'] and candidate_iou>=initial_iou and
        after['feature_heldout_median_px']<=before['feature_heldout_median_px']+.25 and
        after['feature_heldout_p95_px']<=before['feature_heldout_p95_px']+.5)
    report={'before':before,'after':after,'accepted':accepted,'evaluations':result.nfev,
        'rotation_degrees':float(np.degrees(np.linalg.norm(result.x[:3]))),
        'translation_fraction_radius':float(np.linalg.norm(result.x[3:])),
        'feature_observations':len(points),'contour_samples':len(contour),
        'feature_weight':feature_weight,'measurement_resolution':{'width':width,'height':height},
        'initial_mask_iou':initial_iou,'candidate_mask_iou':candidate_iou}
    updated={k:v for k,v in row.items() if not k.startswith('_')}
    if accepted:updated.update(rotation=rotation.tolist(),translation=translation.tolist())
    return updated,report


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['inputs','mesh','model','photo-map','output']:parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--views',default='',help='Optional comma-separated own view names for a small diagnostic')
    parser.add_argument('--hold-every',type=int,default=5,help='Freeze every Nth camera; its silhouette never enters fitting')
    parser.add_argument('--max-evaluations',type=int,default=40)
    parser.add_argument('--feature-weight',type=float,default=10.)
    args=parser.parse_args();cv2.setNumThreads(2)
    if args.hold_every<2 or args.max_evaluations<1 or not np.isfinite(args.feature_weight) or args.feature_weight<=0:parser.error('invalid holdout or evaluation budget')
    import pycolmap
    model=pycolmap.Reconstruction(str(args.model));images={x.name:x for x in model.images.values()}
    mapping={x['capture']:x['source'] for x in json.loads(args.photo_map.read_text())['photos']}
    data=json.loads((args.inputs/'cameras.json').read_text());triangles=read_stl(args.mesh)
    sparse=np.load(args.inputs/'sparse_points.npy',allow_pickle=False);centre=np.median(sparse,axis=0)
    radius=float(np.percentile(np.linalg.norm(sparse-centre,axis=1),90))
    if not np.isfinite(radius) or radius<=1e-10:raise ValueError('invalid own sparse bounds')
    args.output.mkdir(parents=True,exist_ok=False);start=time.monotonic();rows=[];reports=[]
    chosen=set(args.views.split(',')) if args.views else None
    for i,row in enumerate(data['views']):
        if i%args.hold_every==0 or (chosen is not None and row['name'] not in chosen):
            rows.append(row);reports.append({'view':row['name'],'held_out':i%args.hold_every==0,'accepted':False});continue
        image=images[mapping[row['source']]];camera=model.cameras[image.camera_id];pose=image.cam_from_world
        if not np.allclose(pose.rotation.matrix(),row['rotation'],atol=1e-7) or not np.allclose(pose.translation,row['translation'],atol=1e-7):
            raise ValueError('own model and native scene world frames disagree')
        seen=[point for point in image.points2D if point.has_point3D() and point.point3D_id in model.points3D]
        # Deterministic spread, including actual measurements held out from fitting.
        seen=seen[::max(1,len(seen)//500)][:500]
        points=np.array([model.points3D[x.point3D_id].xyz for x in seen])
        rays=np.array([camera.cam_from_img(x.xy) for x in seen])
        observations=rays*np.asarray(row['k'][:2])+np.asarray(row['k'][2:])
        try:updated,report=refine(dict(row,_mask=str(args.inputs/row['mask'])),triangles,points,observations,centre,radius,args.max_evaluations,args.feature_weight)
        except ValueError as error:updated=row;report={'accepted':False,'reason':str(error)}
        rows.append(updated);reports.append(dict(report,view=row['name'],held_out=False));print(json.dumps(reports[-1]),flush=True)
    paths={'sparse_points.npy'}
    for row in rows:paths.update(row[key] for key in ['image','mask','texture_image'] if key in row)
    for name in paths:
        source=args.inputs/name
        if Path(name).is_absolute() or not source.resolve().is_relative_to(args.inputs.resolve()):raise ValueError('input path escapes scene')
        target=args.output/name;target.parent.mkdir(parents=True,exist_ok=True);os.link(source,target)
    (args.output/'cameras.json').write_text(json.dumps(dict(data,views=rows),indent=2)+'\n')
    report={'seconds':time.monotonic()-start,'per_view':reports,'accepted':sum(x['accepted'] for x in reports),
        'reference_used':False,'actual_own_sparse_observations':True,'fixed_intrinsics_and_sparse_structure':True,
        'limits':'Experimental bounded camera-only optimization, not full joint BA. Hull may encode errors from initial cameras. Initial SfM used all images. Holdout camera silhouettes are excluded from optimization; a rebuilt mesh must be checked on them before adoption.'}
    (args.output/'camera-refinement.json').write_text(json.dumps(report,indent=2)+'\n')


if __name__=='__main__':main()
