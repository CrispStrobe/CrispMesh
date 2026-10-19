#!/usr/bin/env python
"""Assess own recovered cameras with an actual silhouette, before Gaussian training.

Sparse reprojection/mask support can pass while thin parts are carved away.
This diagnostic uses the same native defaults and coverage selection as the run.
No reference geometry, supplied pose or hand-labelled part enters the score.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT))
from scripts.photo_regression.capture_pipeline import selected_count
from scripts.photo_regression.stage_cache import sha


def assess(args):
    args.output.mkdir(parents=True,exist_ok=False)
    def run(name,command):
        with (args.output/(name+'.log')).open('w') as log:
            result=subprocess.run(list(map(str,command)),cwd=ROOT,stdout=log,stderr=subprocess.STDOUT,
                env={**os.environ,'RAYON_NUM_THREADS':'2'})
        return result.returncode==0
    def failed(stage):
        report={'status':'failed','stage':stage,'base_mask_iou':None,'reference_used':False}
        (args.output/'report.json').write_text(json.dumps(report,indent=2)+'\n')
        return report
    native=args.native;frontend=args.output/'frontend'
    if not run('native-inputs',[native,'photos','--photos',args.photos,'--calibration',args.calibration,
        '--masks','import:'+str(args.masks),'--cameras','import:'+str(args.model),'--capture','orbit',
        '--output',frontend,'--contrast-gamma','1','--clahe-clip','0','--object-crop-padding',str(args.object_crop_padding),
        '--threads','2','--minimum-free-gib',str(args.minimum_free_gib)]):return failed('native-inputs')
    count=len(json.loads((frontend/'inputs/cameras.json').read_text())['views'])
    if count<8:return failed('too-few-supported-views')
    keep=min(count,selected_count(max(count,args.before_pruning)))
    inputs=frontend/'inputs';fallback=False
    if keep<count:
        selected=args.output/'selected-inputs'
        if run('select-views',[args.python,ROOT/'scripts/photo_regression/select_views.py','--inputs',inputs,
            '--support-report',args.support_report,'--photo-map',frontend/'photo-map.json','--keep',str(keep),
            '--link-inputs','--output',selected]):inputs=selected
        elif 'selection loses direction coverage' in (args.output/'select-views.log').read_text():fallback=True
        else:return failed('select-views')
    base=args.output/'silhouette'
    if not run('silhouette',[native,'run','--inputs',inputs,'--output',base,'--base-surface','silhouette',
        '--threads','2','--minimum-free-gib',str(args.minimum_free_gib),'--no-live-previews','--skip-check','--no-preview']):return failed('silhouette')
    check=args.output/'all-view-check'
    if not run('all-view-check',[native,'check','--inputs',frontend/'inputs','--mesh',base/'mesh/mesh.stl',
        '--output',check,'--check-views',str(count)]):return failed('all-view-check')
    score=json.loads((check/'result.json').read_text())['silhouette_iou_input_masks']['median']
    report={'status':'complete','registered_views':count,'reconstruction_views':len(json.loads((inputs/'cameras.json').read_text())['views']),
        'inputs':str(inputs.relative_to(args.output)),'base_mask_iou':score,'base_mesh_sha256':sha(base/'mesh/mesh.stl'),
        'selection_used_all_views':fallback,'reference_used':False,
        'limits':'Own silhouette agreement is a model-selection diagnostic, not geometric accuracy.'}
    (args.output/'report.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report),flush=True)
    return report


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['native','photos','masks','model','calibration','support-report','output']:
        parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--python',default=sys.executable)
    parser.add_argument('--before-pruning',type=int,required=True)
    parser.add_argument('--object-crop-padding',type=float,default=.15)
    parser.add_argument('--minimum-free-gib',type=float,default=8.8)
    args=parser.parse_args();assess(args)


if __name__=='__main__':main()
