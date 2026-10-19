#!/usr/bin/env python
"""Refine an own cached scene with the same full-view surface check as raw capture."""
import argparse
import json
import math
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time

ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT))
from scripts.photo_regression.capture_pipeline import Cancelled,relay_native_events,select_capture_surface
from scripts.turntable_mesh.dense_events import EventLog,SCHEMA


def run(args):
    tools=json.loads(args.tools.read_text());native=tools['native']
    args.output.mkdir(parents=True,exist_ok=False)
    events=EventLog(args.output/'events.jsonl');start=time.monotonic();timings=[]
    count=len(json.loads(((args.validation_inputs or args.inputs)/'cameras.json').read_text())['views'])
    events.emit('run_started',schema=SCHEMA,device='Mac',inputs=args.inputs.name,
        configuration={'base_surface':args.base_surface,'gaussian':args.gaussian,'gaussian_quality':args.gaussian_quality})
    def execute(name,command,stage=None,native=False):
        if stage:events.emit('stage_started',stage=stage)
        logpath=args.output/(name+'.log');began=time.monotonic();offset=0
        with logpath.open('w') as log:
            child=subprocess.Popen(list(map(str,command)),cwd=ROOT,stdout=log,stderr=subprocess.STDOUT,
                start_new_session=True,env={**os.environ,'RAYON_NUM_THREADS':'2','OMP_NUM_THREADS':'2','OPENBLAS_NUM_THREADS':'2'})
            try:
                while child.poll() is None:
                    if native:offset=relay_native_events(args.output/'reconstruction/events.jsonl',offset,events)
                    if (args.output/'cancel').exists():raise Cancelled('cancelled')
                    if shutil.disk_usage(args.output).free<args.minimum_free_gib*2**30:raise RuntimeError('free-space reserve reached')
                    if time.monotonic()-began>max(1800,20*count):raise RuntimeError('stage deadline exceeded')
                    if logpath.stat().st_size>32*2**20:raise RuntimeError('stage log budget exceeded')
                    time.sleep(.2)
                if native:relay_native_events(args.output/'reconstruction/events.jsonl',offset,events)
                if child.returncode:raise RuntimeError(f'{name} failed; see {name}.log')
            finally:
                if child.poll() is None:
                    os.killpg(child.pid,signal.SIGTERM)
                    try:child.wait(timeout=3)
                    except subprocess.TimeoutExpired:os.killpg(child.pid,signal.SIGKILL);child.wait()
        timings.append({'stage':name,'seconds':time.monotonic()-began})
        if stage:events.emit('stage_finished',stage=stage,seconds=timings[-1]['seconds'])
    try:
        command=[native,'run','--inputs',args.inputs,'--output',args.output/'reconstruction',
            '--base-surface',args.base_surface,'--threads','2','--minimum-free-gib',str(args.minimum_free_gib),'--no-live-previews']
        if args.gaussian:command+=['--splats',args.gaussian_quality]
        for item in args.gaussian_set:command+=['--splats-set',item]
        for item in args.set:command+=['--set',item]
        if args.texture:command+=['--texture']
        execute('reconstruction',command,native=True)
        primary,selection=select_capture_surface(args,native,args.inputs,args.validation_inputs or args.inputs,count,{},execute,events)
        report={'status':'complete','seconds':time.monotonic()-start,'stages':timings,
            'input_kind':'recovered_scene','registered_views':count,
            'reconstruction_views':len(json.loads((args.inputs/'cameras.json').read_text())['views']),
            'base_surface':args.base_surface,'gaussian':args.gaussian,'gaussian_selected':selection['gaussian_selected'],
            'primary_surface':selection['primary_surface'],'surface_choice':selection,'mesh':str(primary),
            'reference_used':False,'original_camera_recovery_repeated':False,'mask_agreement_gate_passed':True}
        (args.output/'pipeline.json').write_text(json.dumps(report,indent=2)+'\n')
        events.emit('run_finished',status='complete',seconds=report['seconds'])
    except BaseException as error:
        events.emit('error',message=str(error))
        events.emit('run_finished',status='cancelled' if isinstance(error,Cancelled) else 'failed',seconds=time.monotonic()-start)
        raise


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['inputs','output','tools']:parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--validation-inputs',type=Path)
    parser.add_argument('--base-surface',choices=['silhouette','stereo'],default='silhouette')
    parser.add_argument('--gaussian',action='store_true')
    parser.add_argument('--gaussian-quality',choices=['capture','fast','standard','best'],default='capture')
    parser.add_argument('--gaussian-set',action='append',default=[])
    parser.add_argument('--set',action='append',default=[])
    parser.add_argument('--texture',action='store_true')
    parser.add_argument('--minimum-free-gib',type=float,default=8.8)
    args=parser.parse_args()
    if not math.isfinite(args.minimum_free_gib) or args.minimum_free_gib<0:parser.error('invalid free-space reserve')
    for item in args.gaussian_set:
        key,sep,value=item.partition('=')
        if not sep or key not in {'steps','crop','mesh','seed'} or not value.isdigit():parser.error('Gaussian overrides require steps/crop/mesh/seed=INTEGER')
    for name in ['inputs','output','tools']:
        setattr(args,name,getattr(args,name).resolve())
    if args.validation_inputs:args.validation_inputs=args.validation_inputs.resolve()
    def cancelled(signum,frame):raise Cancelled('cancelled')
    signal.signal(signal.SIGTERM,cancelled)
    run(args)


if __name__=='__main__':main()
