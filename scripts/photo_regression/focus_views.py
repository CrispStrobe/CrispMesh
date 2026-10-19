#!/usr/bin/env python
"""Optional focus ablation on an own recovered scene; never deletes originals.

Scores contrast-normalized RGB gradients inside the object, at a common object
size. Compares nearby own camera directions and guards coverage. This estimates
relative sharpness, not focus truth: texture, noise and lighting also affect it.
For a single-frequency texture, blur can resemble a contrast change; this
contrast-normalized score cannot distinguish those and must not promise it.
Cameras/sparse bounds are retained, so this does not test camera recovery.
"""
import argparse
import json
import os
from pathlib import Path
import shutil

import cv2
import numpy as np
from PIL import Image


def sharpness(rgb, mask):
    rgb=np.asarray(rgb);mask=np.asarray(mask,dtype=bool)
    if rgb.ndim!=3 or rgb.shape[2]!=3 or mask.shape!=rgb.shape[:2] or not mask.any():
        raise ValueError('RGB image and nonempty matching own mask required')
    y,x=np.where(mask);left,top,right,bottom=x.min(),y.min(),x.max()+1,y.max()+1
    pad=int(max(right-left,bottom-top)*.05)
    left=max(0,left-pad);top=max(0,top-pad);right=min(rgb.shape[1],right+pad);bottom=min(rgb.shape[0],bottom+pad)
    image=rgb[top:bottom,left:right];region=mask[top:bottom,left:right]
    scale=512/max(image.shape[:2]);size=(max(8,round(image.shape[1]*scale)),max(8,round(image.shape[0]*scale)))
    image=cv2.resize(image,size,interpolation=cv2.INTER_AREA if scale<1 else cv2.INTER_LINEAR)
    region=cv2.resize(region.astype('uint8'),size,interpolation=cv2.INTER_NEAREST)
    core=cv2.erode(region,np.ones((5,5),'uint8'))>0
    if not core.any():return None
    gray=cv2.cvtColor(image,cv2.COLOR_RGB2GRAY).astype('float32')/255
    low,high=np.percentile(gray[core],[10,90]);gray=np.clip((gray-low)/max(high-low,.1),0,1)
    smooth=cv2.GaussianBlur(gray,(0,0),.6)
    dx=cv2.Sobel(smooth,cv2.CV_32F,1,0,ksize=3,scale=1/8)
    dy=cv2.Sobel(smooth,cv2.CV_32F,0,1,ksize=3,scale=1/8)
    return float(np.mean((dx*dx+dy*dy)[core]))


def select(scores, directions, ratio=.65, maximum_gap=20.):
    scores=np.asarray([np.nan if value is None else value for value in scores],dtype=float)
    directions=np.asarray(directions,dtype=float)
    if len(scores)<8 or directions.shape!=(len(scores),3) or not np.isfinite(directions).all():
        raise ValueError('at least eight scores and finite camera directions required')
    if not np.isfinite(ratio) or not 0<ratio<1 or not np.isfinite(maximum_gap) or not 0<maximum_gap<180:
        raise ValueError('invalid focus ratio or direction coverage bound')
    lengths=np.linalg.norm(directions,axis=1)
    if np.any(lengths<=1e-12) or np.any(np.isfinite(scores)&(scores<0)):
        raise ValueError('undefined direction or negative sharpness')
    directions=directions/lengths[:,None]
    angles=np.degrees(np.arccos(np.clip(directions@directions.T,-1,1)))
    relatives=[]
    for i,score in enumerate(scores):
        neighbours=[j for j in range(len(scores)) if j!=i and angles[i,j]<=maximum_gap and np.isfinite(scores[j])]
        reference=float(np.percentile(scores[neighbours],75)) if len(neighbours)>=2 else 0
        relatives.append(float(score/reference) if reference>1e-12 and np.isfinite(score) else None)
    kept=set(range(len(scores)));excluded=[];protected=[]
    for i in sorted(kept,key=lambda i:relatives[i] if relatives[i] is not None else float('inf')):
        if relatives[i] is None or relatives[i]>=ratio:continue
        trial=kept-{i};removed=sorted(set(range(len(scores)))-trial)
        if len(trial)<8 or float(angles[np.ix_(removed,sorted(trial))].min(1).max())>maximum_gap:
            protected.append(i);continue
        kept=trial;excluded.append(i)
    return sorted(kept),{'relative_sharpness':relatives,'excluded_indices':excluded,'coverage_protected_indices':protected,
        'relative_gradient_threshold':ratio,'maximum_direction_gap_degrees':maximum_gap,'minimum_nearby_views':2}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--inputs',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--relative-gradient',type=float,default=.65)
    parser.add_argument('--maximum-direction-gap',type=float,default=20.)
    args=parser.parse_args();cv2.setNumThreads(2)
    data=json.loads((args.inputs/'cameras.json').read_text());rows=data['views']
    sparse=np.load(args.inputs/'sparse_points.npy',allow_pickle=False)
    if sparse.ndim!=2 or sparse.shape[1]!=3 or not len(sparse) or not np.isfinite(sparse).all():raise ValueError('finite own sparse points required')
    def inside(name):
        path=args.inputs/name
        if Path(name).is_absolute() or not path.resolve().is_relative_to(args.inputs.resolve()):raise ValueError('input must stay inside scene')
        return path
    scores=[sharpness(np.array(Image.open(inside(row.get('texture_image',row['image']))).convert('RGB')),
        np.array(Image.open(inside(row['mask'])).convert('L'))>127) for row in rows]
    rotations=np.array([row['rotation'] for row in rows]);translations=np.array([row['translation'] for row in rows])
    if rotations.shape!=(len(rows),3,3) or translations.shape!=(len(rows),3) or not np.isfinite(rotations).all() or not np.isfinite(translations).all():raise ValueError('finite own cameras required')
    if np.max(np.abs(rotations@rotations.transpose(0,2,1)-np.eye(3)))>1e-5 or np.max(np.abs(np.linalg.det(rotations)-1))>1e-5:raise ValueError('rigid own cameras required')
    directions=-np.einsum('nji,nj->ni',rotations,translations)-np.median(sparse,axis=0)
    kept,report=select(scores,directions,args.relative_gradient,args.maximum_direction_gap)
    selected=[rows[i] for i in kept];paths={'sparse_points.npy'}
    for row in selected:paths.update(row[key] for key in ['image','mask','texture_image'] if key in row)
    args.output.mkdir(parents=True,exist_ok=False)
    # Hardlinks keep immutable scene bytes without duplicating original photos.
    for name in sorted(paths):
        source=inside(name);target=args.output/name;target.parent.mkdir(parents=True,exist_ok=True)
        if source.stat().st_dev==args.output.stat().st_dev:os.link(source,target)
        else:shutil.copy2(source,target)
    (args.output/'cameras.json').write_text(json.dumps(dict(data,views=selected),indent=2)+'\n')
    report.update(input_views=len(rows),kept_views=len(kept),own_cameras_retained=True,reference_used=False,
        photos=[{'source':row['source'],'sharpness':scores[i],'relative_sharpness':report['relative_sharpness'][i],'excluded':i not in kept} for i,row in enumerate(rows)],
        limits='Optional fixed-camera ablation. Not reliable focus classification or proof of better camera recovery. Check every original view too.')
    (args.output/'focus-selection.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({'input_views':len(rows),'kept_views':len(kept)}))


if __name__=='__main__':main()
