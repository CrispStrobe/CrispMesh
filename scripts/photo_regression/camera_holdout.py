"""Photo-only camera diagnostic: remove complete match components before recovery.

Run partition, recover cameras with its training match cache, then evaluate the
reserved tracks. This avoids reserving individual edges of a training track.
No dataset poses are consumed. Incorrect matches remain in the diagnostic;
report counts and percentiles, do not treat this as a replacement quality gate.
"""
import argparse
import json
from pathlib import Path
import numpy as np


def partition(matches, every=10):
    parent={}
    def find(x):
        parent.setdefault(x,x)
        while parent[x]!=x:
            parent[x]=parent[parent[x]];x=parent[x]
        return x
    for pair in matches['pairs']:
        for a,b in pair['matches']:
            x,y=find((pair['first'],a)),find((pair['second'],b));parent[x]=y
    groups={}
    for x in sorted(parent):groups.setdefault(find(x),[]).append(x)
    groups=sorted(groups.values())
    # Split entire components regardless of ambiguity; report ambiguous ones.
    reserved=[g for i,g in enumerate(groups) if i%every==0]
    nodes={tuple(x) for g in reserved for x in g}
    train={**matches,'pairs':[]}
    for p in matches['pairs']:
        train['pairs'].append({**p,'matches':[[a,b] for a,b in p['matches'] if (p['first'],a) not in nodes]})
    tracks=[g for g in reserved if len(g)>=4 and len({v for v,f in g})==len(g)]
    return train,{'tracks':tracks,'reserved_components':len(reserved),'reserved_nodes':len(nodes),
                  'ambiguous_or_short_components':len(reserved)-len(tracks),'split':'every tenth complete connected component, sorted by feature IDs'}


def evaluate(matches, held, rows, lens, foreground_masks=None, selection_rows=None):
    import cv2
    k=np.array([[lens['fx'],0,lens['cx']],[0,lens['fy'],lens['cy']],[0,0,1]])
    distortion=np.array([lens['k1'],lens['k2'],0,0,lens['k3']])
    points=[cv2.undistortPoints(np.array(p,float).reshape(-1,1,2),k,distortion).reshape(-1,2) for p in matches['keypoints']]
    if len(matches['keypoints']) != len(rows):
        raise ValueError('match cache and recovered cameras must have the same view count')
    if selection_rows is not None and [r['name'] for r in selection_rows] != [r['name'] for r in rows]:
        raise ValueError('selection cameras must have the same ordered view names')
    groups={name:[] for name in ['foreground','background','outside_mask']}
    verified_groups={name:[] for name in groups}
    if foreground_masks is not None:
        if len(foreground_masks) != len(rows):
            raise ValueError('one foreground mask is required per recovered camera')
        for row,mask in zip(rows,foreground_masks):
            if mask.ndim != 2 or mask.shape != (row['height'],row['width']):
                raise ValueError('foreground mask dimensions must match recovered camera: '+row['name'])
    def triangulate(cameras,fit):
        design=[]
        for v,f in fit:
            p=np.column_stack((cameras[v]['rotation'],cameras[v]['translation']));x,y=points[v][f]
            design.extend([x*p[2]-p[0],y*p[2]-p[1]])
        _,_,vt=np.linalg.svd(design);h=vt[-1]
        return h[:3]/h[3] if abs(h[3])>=1e-12 else None
    def verify(cameras,world,fit):
        if world is None or len(fit)<3:return False
        fit_errors=[]
        for v,f in fit:
            q=np.array(cameras[v]['rotation'])@world+cameras[v]['translation']
            fit_errors.append(float(np.linalg.norm((q[:2]/q[2]-points[v][f])*[lens['fx'],lens['fy']])) if q[2]>0 else float('inf'))
        # Selection uses only the fitting observations, never held-out errors.
        return max(fit_errors)<1.0
    errors=[];per_view=[[] for r in rows];behind=0;failed=0;fit_verified=[];fit_verified_tracks=0
    selected_samples=0;invalid_selected=0
    for track in held['tracks']:
        fit,test=track[::2],track[1::2]
        world=triangulate(rows,fit)
        selected_world=world if selection_rows is None else triangulate(selection_rows,fit)
        verified=verify(rows if selection_rows is None else selection_rows,selected_world,fit)
        fit_verified_tracks+=int(verified)
        if verified:selected_samples+=len(test)
        if world is None:
            failed+=1
            if verified:invalid_selected+=len(test)
            continue
        for v,f in test:
            q=np.array(rows[v]['rotation'])@world+rows[v]['translation']
            if q[2]<=0 or not np.isfinite(q).all():
                behind+=1
                if verified:invalid_selected+=1
                continue
            residual=(q[:2]/q[2]-points[v][f])*[lens['fx'],lens['fy']]
            e=float(np.linalg.norm(residual));errors.append(e);per_view[v].append(e)
            if verified:fit_verified.append(e)
            if foreground_masks is not None:
                fx,fy,cx,cy=rows[v]['k']
                pixel=points[v][f]*[fx,fy]+[cx,cy]
                # Engine intrinsics map pixel centres to x+.5, y+.5.
                x,y=np.floor(pixel).astype(int)
                mask=foreground_masks[v]
                group='outside_mask' if not (0<=x<mask.shape[1] and 0<=y<mask.shape[0]) else ('foreground' if mask[y,x]!=0 else 'background')
                groups[group].append(e)
                if verified:verified_groups[group].append(e)
    def summary(e):
        return {'count':len(e),'median':float(np.median(e)) if e else None,'p95':float(np.percentile(e,95)) if e else None,'fraction_below_1px':float(np.mean(np.array(e)<1)) if e else None}
    result={'schema':'crispmesh_camera_holdout_v1','reference_used':False,'training_tracks_must_exclude_reserved_components':True,
            'reserved_tracks':len(held['tracks']),'failed_triangulations':failed,'behind_camera_observations':behind,
            'held_out_reprojection_pixels':summary(errors),'per_view':[{'name':r['name'],**summary(e)} for r,e in zip(rows,per_view)],
            'fit_verified_tracks':fit_verified_tracks,'fit_verified_held_out_pixels':summary(fit_verified),
            'fit_verified_selection':'candidate fitting observations' if selection_rows is None else 'fixed selection cameras, fitting observations only',
            'fit_verified_selected_observations':selected_samples,'fit_verified_invalid_observations':invalid_selected,
            'limits':['Report is meaningful only when cameras were recovered using the partitioned training cache.',
                      'False feature matches are included; this diagnostic does not establish dense or local facial accuracy.',
                      'Reserved tracks are triangulated using alternating observations; remaining observations score the cameras.']}
    if selection_rows is not None:
        result['limits'].append('Both candidate and selection cameras must be recovered using the partitioned training cache. Fixed eligibility uses selection cameras and fitting observations only. Candidate points are retriangulated independently. Compare fixed-subset errors only with zero invalid selected observations; invalid observations must not be silently dropped.')
    if foreground_masks is not None:
        result['photo_mask_groups']={name:{'held_out_reprojection_pixels':summary(groups[name]),'fit_verified_held_out_pixels':summary(verified_groups[name])} for name in groups}
        result['limits'].append('Foreground/background labels use photo-derived masks only; mask errors can misclassify observations. Groups do not change fitting or track selection.')
    return result


def main():
    p=argparse.ArgumentParser(description=__doc__);sub=p.add_subparsers(dest='command',required=True)
    for name in ['partition','evaluate']:
        q=sub.add_parser(name);q.add_argument('--matches',type=Path,required=True);q.add_argument('--output',type=Path,required=True)
        if name=='evaluate':
            q.add_argument('--reserved',type=Path,required=True);q.add_argument('--cameras',type=Path,required=True);q.add_argument('--calibration',type=Path,required=True);q.add_argument('--foreground-masks',action='store_true',help='group held-out errors using the recovered cameras photo-derived masks')
            q.add_argument('--selection-cameras',type=Path,help='own baseline cameras that freeze verified-track eligibility using fitting observations only; never supplied dataset poses')
    a=p.parse_args();matches=json.loads(a.matches.read_text())
    if a.output.exists():raise ValueError('output exists')
    a.output.mkdir(parents=True)
    if a.command=='partition':
        train,held=partition(matches)
        (a.output/'training-matches.json').write_text(json.dumps(train,separators=(',',':')))
        (a.output/'reserved-tracks.json').write_text(json.dumps(held,indent=2))
        print({k:v for k,v in held.items() if k!='tracks'})
    else:
        rows=json.loads(a.cameras.read_text())['views'];masks=None
        if a.foreground_masks:
            import cv2
            masks=[]
            for row in rows:
                path=Path(row['mask'])
                if not path.is_absolute():path=a.cameras.parent/path
                mask=cv2.imread(str(path),cv2.IMREAD_GRAYSCALE)
                if mask is None:raise ValueError('cannot read foreground mask: '+str(path))
                masks.append(mask)
        selection_rows=json.loads(a.selection_cameras.read_text())['views'] if a.selection_cameras else None
        result=evaluate(matches,json.loads(a.reserved.read_text()),rows,json.loads(a.calibration.read_text()),masks,selection_rows)
        (a.output/'result.json').write_text(json.dumps(result,indent=2));print(json.dumps(result['held_out_reprojection_pixels']))

if __name__=='__main__':main()
