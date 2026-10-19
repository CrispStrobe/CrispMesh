#!/usr/bin/env python
"""Own masked RootSIFT overlap graph for a separate hybrid learned-match path.

Uses the existing OpenCV dependency, no additional model or pose prior. Only
the strongest four neighbours per photo are added to the learned pair graph.
"""
import argparse
import json
import time
import sys
from pathlib import Path

import cv2
import numpy as np
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from scripts.photo_regression.learned_matches import candidate_graph, square_crop, source_keypoints, sha


def mutual_pairs(first, second, ratio=.8):
    if len(first)<2 or len(second)<2:return []
    matcher=cv2.BFMatcher(cv2.NORM_L2)
    def good(a,b):return {m.queryIdx:m.trainIdx for m,n in matcher.knnMatch(a,b,k=2) if m.distance<ratio*n.distance}
    forward,backward=good(first,second),good(second,first)
    return [[a,b] for a,b in sorted(forward.items()) if backward.get(b)==a]


def strongest_edges(pairs,count,neighbours=4,minimum=12):
    ranked=[[] for _ in range(count)]
    for row in pairs:
        a,b=row['first'],row['second'];score=len(row['matches'])
        if score>=minimum:ranked[a].append((score,b));ranked[b].append((score,a))
    selected=set()
    for a,rows in enumerate(ranked):
        for _,b in sorted(rows,key=lambda r:(-r[0],r[1]))[:neighbours]:selected.add(tuple(sorted([a,b])))
    return [row for row in pairs if (row['first'],row['second']) in selected]


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for key in ['template','photos','masks','output']:parser.add_argument('--'+key,type=Path,required=True)
    args=parser.parse_args();cv2.setNumThreads(2);start=time.monotonic()
    template=json.loads(args.template.read_text());names=template['names']
    if not 8<=len(names)<=1536 or any(Path(name).name!=name for name in names):raise ValueError('invalid original photo names/count')
    extractor=cv2.SIFT_create(nfeatures=800,contrastThreshold=.015)
    descriptors=[];keys=[];pooled=[];ledger=[]
    for i,name in enumerate(names):
        photo=args.photos/name;mask_path=args.masks/(name+'.png')
        with Image.open(photo) as image,Image.open(mask_path) as mask:
            mask=mask.convert('L');box=square_crop(np.asarray(mask),.3,True)
            gray=np.asarray(image.convert('L').crop(box).resize((768,768),Image.Resampling.LANCZOS))
            support=np.asarray(mask.crop(box).resize((768,768),Image.Resampling.NEAREST))
        points,desc=extractor.detectAndCompute(gray,support)
        if desc is None:desc=np.empty((0,128),'float32')
        desc=np.sqrt(desc/np.maximum(desc.sum(1,keepdims=True),1e-12)).astype('float32')
        coordinates=np.asarray([p.pt for p in points],dtype='float32').reshape(-1,2)
        keys.append(source_keypoints(coordinates,box,768).tolist());descriptors.append(desc)
        pooled.append(desc.mean(0) if len(desc) else np.zeros(128,'float32'))
        ledger.append({'name':name,'photo_sha256':sha(photo),'mask_sha256':sha(mask_path),'crop':box})
        print('SIFT overlap features',i+1,'/',len(names),len(desc),flush=True)
    # Exhaustive cheap descriptor scores for small sets; a bounded appearance /
    # temporal proposal graph for larger ones. No fixed-angle assumption.
    candidates=[(a,b) for a in range(len(names)) for b in range(a+1,len(names))] if len(names)<=192 else candidate_graph(pooled,32,8)
    pairs=[]
    for i,(a,b) in enumerate(candidates):
        matches=mutual_pairs(descriptors[a],descriptors[b])
        if len(matches)>=12:pairs.append({'first':a,'second':b,'matches':matches})
        if i%100==0:print('SIFT overlap pairs',i+1,'/',len(candidates),flush=True)
    selected=strongest_edges(pairs,len(names))
    result={**template,'keypoints':keys,'pairs':selected,'note':'Own masked RootSIFT overlap proposals; no supplied pose, depth, geometry or angle.'}
    args.output.write_text(json.dumps(result)+'\n')
    args.output.with_suffix('.report.json').write_text(json.dumps({'photos':len(names),'candidate_pairs':len(candidates),'retained_pairs':len(selected),'seconds':time.monotonic()-start,'source':'existing OpenCV SIFT/BFMatcher APIs; original adapter; no upstream algorithm source copied','reference_used':False,'inputs':ledger},indent=2)+'\n')


if __name__=='__main__':main()
