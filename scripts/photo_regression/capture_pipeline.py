#!/usr/bin/env python
"""Desktop real-capture route from original photos to silhouette / optional Gaussian.

External permissive model sources/weights are host-configured and hash checked by
stage runners. No supplied poses, reference geometry or per-batch pixel/angle tuning.
The mobile app can use this engine over HTTP; this is not on-phone Python.
"""
import argparse
import hashlib
import json
import math
import os
import re
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
from scripts.turntable_mesh.dense_events import EventLog, SCHEMA
from scripts.photo_regression.stage_cache import StageCache, sha, inventory


class Cancelled(RuntimeError): pass


def selected_count(count, excluded_fraction=.08):
    if not 8 <= count <= 1536 or not 0 <= excluded_fraction < .5:
        raise ValueError('invalid recovered view count/selection budget')
    return max(8, count-math.ceil(count*excluded_fraction))


def registered_count(report):
    return max((row['cameras'] for row in report.get('models',[])),default=0)


def camera_acceptable(report, support, accepted):
    fraction=support.get('median_inside_fraction',0.)
    return registered_count(report)>=math.ceil(.8*accepted) and math.isfinite(fraction) and fraction>=.85


def geometry_acceptable(score):
    return isinstance(score,(int,float)) and not isinstance(score,bool) and math.isfinite(score) and .8<=score<=1.


def camera_search_order(preferred):
    orders={'hybrid':('hybrid','tight','wide'),'tight':('tight','wide','hybrid'),'wide':('wide','hybrid','tight')}
    if preferred not in orders:raise ValueError('unknown camera search path')
    return orders[preferred]


def run(args):
    cfg = json.loads(args.tools.read_text())
    required = ['native', 'python_torch', 'python_geometry', 'lightglue_source', 'extractor', 'matcher']
    refine_masks=(args.refine_masks and not args.masks) or args.refine_imported_masks
    if refine_masks:required+=['sam_source','sam_checkpoint']
    if args.foreground_provider=='app' and not args.masks:required.append('foreground')
    if any(not isinstance(cfg.get(key),str) or not cfg[key] or not Path(cfg[key]).exists() for key in required):
        raise ValueError('capture tools configuration requires installed native engine, interpreters and pinned permissive model files')
    args.output.mkdir(parents=True, exist_ok=args.resume)
    recipe={'photos':{p.name:sha(p) for p in sorted(args.photos.iterdir()) if p.suffix.lower() in {'.jpg','.jpeg','.png'}},
        'tools_sha256':sha(args.tools),'calibration':sha(args.calibration) if args.calibration else None,
        'fov':args.fov,'gaussian':args.gaussian,'quality':args.gaussian_quality,'settings':args.gaussian_set,
        'base_surface':args.base_surface,'texture':args.texture,
        'foreground_provider':args.foreground_provider,'foreground_binary':sha(cfg['foreground']) if args.foreground_provider=='app' and not args.masks else None,
        'shared_crop':args.shared_crop,
        'imported_masks':{p.name:sha(p) for p in sorted(args.masks.glob('*.png'))} if args.masks else None,
        'refine_imported_masks':args.refine_imported_masks,'object_crop_padding':args.object_crop_padding,'capture_groups':args.capture_groups,
        'refine_masks':args.refine_masks,'camera_search':args.camera_search,
        'source':{p.name:sha(p) for p in Path(__file__).parent.glob('*.py') if not p.name.startswith('test_')},
        'vision_source':sha(Path(__file__).with_name('foreground_instances.swift')),'native_binary':sha(cfg['native'])}
    fingerprint=hashlib.sha256(json.dumps(recipe,sort_keys=True).encode()).hexdigest()
    if args.capture_groups and sum(map(int,args.capture_groups.split(',')))!=len(recipe['photos']):raise ValueError('capture group counts do not match the input photos')
    cache=StageCache(args.output,fingerprint,args.resume,args.adopt_completed)
    (args.output/'capture-recipe.json').write_text(json.dumps(recipe,indent=2)+'\n')
    old_timings=json.loads((args.output/'timings.json').read_text()) if args.resume and (args.output/'timings.json').exists() else []
    events = EventLog(args.output/'events.jsonl'); start = time.monotonic(); timings = old_timings.copy()
    events.emit('run_started', schema=SCHEMA, device='Mac', configuration={'base_surface': args.base_surface, 'gaussian': args.gaussian, 'gaussian_quality':args.gaussian_quality, 'gaussian_overrides':args.gaussian_set,'camera_search':args.camera_search, 'foreground_margin_pixels':3, 'excluded_view_fraction':.08}, inputs=args.photos.name)
    def relay(path, offset):
        if not path.exists(): return offset
        with path.open() as stream:
            stream.seek(offset)
            while True:
                position = stream.tell(); line = stream.readline()
                if not line.endswith('\n'): return position
                row = json.loads(line)
                kind = row.pop('type'); row.pop('time', None)
                if kind in ['run_started','run_finished']: continue
                if kind == 'artifact':
                    source = Path(row.get('path',''))
                    if source.is_absolute() or '..' in source.parts: continue
                    row['path'] = 'reconstruction/'+str(source)
                events.emit(kind, **row)
    declared={
        'vision-build':['foreground-instances'],'vision':['instances'],'prepare':['prepared'],
        'features':['features/matches.json','features/report.json'],'refine-masks':['refined-masks'],
        'validate-masks':['validated-masks'],'filter-matches':['filtered-matches.json'],
        'recover-cameras':['cameras'],'audit':['sparse-support.json'],'native-inputs':['frontend'],
        'features-context':['features-context/matches.json','features-context/report.json'],
        'filter-context':['filtered-context-matches.json'],'recover-context':['cameras-context'],
        'audit-primary':['sparse-support-primary.json'],'audit-context':['sparse-support-context.json'],
        'sift-overlap':['sift-overlap.json','sift-overlap.report.json'],
        'features-hybrid':['features-hybrid/matches.json','features-hybrid/report.json'],
        'filter-hybrid':['filtered-hybrid-matches.json'],'recover-hybrid':['cameras-hybrid'],'audit-hybrid':['sparse-support-hybrid.json'],
        'select-views':['selected-inputs'],'reconstruction':['reconstruction'],'all-view-check':['all-view-check']}
    used_stages=set()
    def execute(name, command, stage=None, native=False, timeout=None):
        used_stages.add(name)
        if timeout is None:timeout=max(1800,20*len(recipe['photos']))
        outputs=declared.get(name,[])
        if args.resume and cache.hit(name,command,outputs):
            if stage: events.emit('stage_finished',stage=stage,seconds=0,cached=True)
            return
        if args.resume:
            for relative in outputs:
                path=args.output/relative
                if path.exists():
                    # Interrupted products are reproducible, not original photos.
                    receipt=args.output/(name+'-discarded.json')
                    receipt.write_text(json.dumps({'path':relative,'reason':'incomplete or changed stage; re-created from original inputs'})+'\n')
                    if path.is_symlink() or path.is_file():path.unlink()
                    else:shutil.rmtree(path)
        if (args.output/'cancel').exists(): raise Cancelled('cancelled')
        if shutil.disk_usage(args.output).free < args.minimum_free_gib*2**30: raise RuntimeError('free-space reserve reached')
        if stage: events.emit('stage_started', stage=stage)
        t = time.monotonic(); logpath=args.output/(name+'.log'); offset=0; progress_offset=0
        env={**os.environ, 'RAYON_NUM_THREADS':'2', 'OMP_NUM_THREADS':'2','OPENBLAS_NUM_THREADS':'2','VECLIB_MAXIMUM_THREADS':'2'}
        with logpath.open('w') as log:
            child=subprocess.Popen(list(map(str,command)), cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                while child.poll() is None:
                    if native: offset=relay(args.output/'reconstruction/events.jsonl',offset)
                    elif name.startswith('features'):
                        with logpath.open() as progress:
                            progress.seek(progress_offset)
                            while True:
                                position=progress.tell(); line=progress.readline()
                                if not line.endswith('\n'):
                                    progress_offset=position; break
                                match=re.fullmatch(r'(features|pairs) (\d+) / (\d+)\s*(?:\d+)?\n',line)
                                if match:
                                    phase,current,total=match.groups();fraction=int(current)/max(1,int(total))
                                    events.emit('progress',stage='cameras',fraction=(.4+.5*fraction if phase=='pairs' else .4*fraction),message='Matching photos' if phase=='pairs' else 'Extracting photo features')
                    if (args.output/'cancel').exists(): raise Cancelled('cancelled')
                    if time.monotonic()-t>timeout: raise RuntimeError('stage deadline exceeded')
                    if shutil.disk_usage(args.output).free < args.minimum_free_gib*2**30: raise RuntimeError('free-space reserve reached')
                    if logpath.stat().st_size > 32*2**20: raise RuntimeError('stage log budget exceeded')
                    time.sleep(.2)
                if native: relay(args.output/'reconstruction/events.jsonl',offset)
                if child.returncode: raise RuntimeError(f'{name} failed; see {logpath.name}')
            finally:
                if child.poll() is None:
                    os.killpg(child.pid, signal.SIGTERM)
                    try: child.wait(timeout=3)
                    except subprocess.TimeoutExpired: os.killpg(child.pid, signal.SIGKILL); child.wait()
        cache.record(name,command,outputs)
        timings[:]=[row for row in timings if row['stage']!=name]
        timings.append({'stage':name,'seconds':time.monotonic()-t})
        (args.output/'timings.json').write_text(json.dumps(timings,indent=2)+'\n')
        if stage: events.emit('stage_finished',stage=stage,seconds=timings[-1]['seconds'])
    def tool(name, interpreter): return [cfg[interpreter], ROOT/'scripts/photo_regression'/name]
    try:
        # Normalize orientation before Vision; upright originals stay byte-exact.
        from PIL import Image, ImageOps
        originals=args.output/'upright'; originals.mkdir(exist_ok=args.resume)
        for file in sorted(args.photos.iterdir()):
            if file.suffix.lower() not in {'.jpg','.jpeg','.png'}: continue
            with Image.open(file) as image:
                target=originals/file.name
                if target.exists():
                    if image.getexif().get(274,1)==1 and sha(target)!=sha(file):raise ValueError('upright original differs on resume')
                    continue
                if image.getexif().get(274,1)==1: os.link(file,target) if file.stat().st_dev==originals.stat().st_dev else shutil.copy2(file,target)
                else: ImageOps.exif_transpose(image).convert('RGB').save(target,quality=97)
        if not 8 <= len(list(originals.iterdir())) <=1536: raise ValueError('expected8..1536 original photos')
        if not args.masks:
            adapter=cfg['foreground'] if args.foreground_provider=='app' else args.output/'foreground-instances'
            if args.foreground_provider=='independent':execute('vision-build',['swiftc','-O',ROOT/'scripts/photo_regression/foreground_instances.swift','-o',adapter])
            execute('vision',[adapter,originals,args.output/'instances'],stage='masks')
        command=tool('prepare_capture.py','python_torch')+['--photos',originals,'--output',args.output/'prepared']
        command+=['--masks',args.masks] if args.masks else ['--instances',args.output/'instances']
        if args.foreground_provider=='app':command+=['--maximum-foreground-fraction','.9']
        if args.shared_crop:command+=['--shared-crop']
        if args.calibration: command+=['--calibration',args.calibration]
        if args.fov is not None: command+=['--fov',str(args.fov)]
        execute('prepare',command)
        prepared=args.output/'prepared'; photos=prepared/'photos'; coarse=prepared/'masks'
        feature_cache=args.data_root/'capture-feature-cache' if args.data_root else args.output/'features'
        masks=coarse
        if refine_masks:
            execute('refine-masks',tool('refine_masks.py','python_torch')+['--photos',photos,'--masks',coarse,'--source',cfg['sam_source'],'--checkpoint',cfg['sam_checkpoint'],'--output',args.output/'refined-masks'],stage='masks')
            execute('validate-masks',tool('validate_masks.py','python_torch')+['--photos',photos,'--coarse-masks',coarse,'--refined-masks',args.output/'refined-masks','--output',args.output/'validated-masks'])
            masks=args.output/'validated-masks'
        cameras_started=time.monotonic();events.emit('stage_started',stage='cameras')
        accepted=len(json.loads((prepared/'template.json').read_text())['names'])
        def camera_support(folder, report, tag):
            if not report.get('selected_model'):return {}
            target=args.output/('sparse-support-'+tag+'.json')
            execute('audit-'+tag,tool('sparse_silhouettes.py','python_geometry')+['--model',folder/report['selected_model'],'--masks',masks,'--output',target])
            return json.loads(target.read_text())
        # Same retained algorithms, configurable search order. Hybrid uses
        # independent RootSIFT overlap proposals plus learned correspondences.
        # No motor angle, named photo or reference pose enters any path.
        specs={
            'tight':('features','filtered-matches.json','recover-cameras','cameras','primary',[]),
            'wide':('features-context','filtered-context-matches.json','recover-context','cameras-context','context',['--padding','.30','--fit-context','--neighbors','12','--temporal-radius','4']),
            'hybrid':('features-hybrid','filtered-hybrid-matches.json','recover-hybrid','cameras-hybrid','hybrid',['--padding','.30','--fit-context','--reuse-overlap'])}
        candidates=[];attempts=[];evaluated=[];chosen=None
        for mode in camera_search_order(args.camera_search):
            feature_name,match_name,recovery_name,camera_name,tag,extra=specs[mode]
            template=prepared/'template.json'
            if mode=='hybrid':
                events.emit('progress',stage='cameras',fraction=0,message='Trying hybrid feature overlap')
                template=args.output/'sift-overlap.json'
                execute('sift-overlap',tool('sift_overlap.py','python_geometry')+['--template',prepared/'template.json','--photos',photos,'--masks',coarse,'--output',template])
            elif mode=='wide':events.emit('progress',stage='cameras',fraction=0,message='Trying wider feature context')
            features=args.output/feature_name
            execute(feature_name,tool('learned_matches.py','python_torch')+['--source',cfg['lightglue_source'],'--extractor',cfg['extractor'],'--matcher',cfg['matcher'],'--template',template,'--photos',photos,'--masks',coarse,'--output',features,*extra,'--cache-root',feature_cache])
            matches=args.output/match_name
            filter_name={'tight':'filter-matches','wide':'filter-context','hybrid':'filter-hybrid'}[mode]
            execute(filter_name,tool('filter_matches.py','python_torch')+['--matches',features/'matches.json','--masks',masks,'--margin-pixels','3','--output',matches])
            folder=args.output/camera_name
            try:
                execute(recovery_name,tool('colmap_recover.py','python_geometry')+['--matches',matches,'--photos',photos,'--output',folder,'--calibration','radial','--minimum-observations',str(20 if accepted>=25 else 8)])
            except Cancelled:
                raise
            except RuntimeError:
                report_path=folder/'report.json'
                if not report_path.is_file() or json.loads(report_path.read_text()).get('selected_model') is not None:raise
            report=json.loads((folder/'report.json').read_text())
            support=camera_support(folder,report,tag)
            attempts.append((folder,report,support))
            candidates.append({'path':camera_name,'registered':registered_count(report),'accepted':accepted,'sparse_mask_support':support.get('median_inside_fraction'),'removed_weak_views':len(report.get('removed_sparse_views',[]))})
            if camera_acceptable(report,support,accepted):
                assessment_name='assess-'+tag;evaluation=args.output/('assessment-'+tag)
                declared[assessment_name]=[evaluation.name]
                execute(assessment_name,tool('evaluate_capture_cameras.py','python_geometry')+[
                    '--native',cfg['native'],'--photos',photos,'--masks',masks,'--model',folder/report['selected_model'],
                    '--calibration',folder/'own-estimated-lens.json','--support-report',args.output/('sparse-support-'+tag+'.json'),
                    '--before-pruning',str(report.get('cameras_before_support_pruning',registered_count(report))),
                    '--object-crop-padding',str(args.object_crop_padding),'--minimum-free-gib',str(args.minimum_free_gib),'--output',evaluation])
                assessment=json.loads((evaluation/'report.json').read_text())
                score=assessment.get('base_mask_iou')
                candidates[-1]['base_mask_iou']=score
                if geometry_acceptable(score):
                    candidate=(folder,report,support,assessment,evaluation);evaluated.append(candidate)
                    # Prefer a strong silhouette; otherwise compare retained camera alternatives.
                    if score>=.85:
                        chosen=candidate;break
        if chosen is None and evaluated:
            chosen=max(evaluated,key=lambda row:row[3]['base_mask_iou'])
        cameras,camera_report,support=chosen[:3] if chosen else max(attempts,key=lambda row:registered_count(row[1])*row[2].get('median_inside_fraction',0.))
        (args.output/'camera-path.json').write_text(json.dumps({'selected':cameras.name,'quality_accepted':chosen is not None,'search_order':list(camera_search_order(args.camera_search)),'candidates':candidates,'minimum_registered_fraction':.8,'minimum_sparse_mask_support':.85,'minimum_base_mask_iou':.8,'preferred_base_mask_iou':.85,'reference_used':False},indent=2)+'\n')
        events.artifact('report',args.output/'camera-path.json','Camera recovery')
        if not camera_report.get('selected_model'):raise ValueError('no connected camera model recovered')
        if chosen is None:raise ValueError('Camera recovery is inconsistent with the object masks. Review the camera report and photos.')
        events.emit('stage_finished',stage='cameras',seconds=time.monotonic()-cameras_started)
        evaluation=chosen[4];assessment=chosen[3]
        native=cfg['native'];frontend=evaluation/'frontend';inputs=evaluation/assessment['inputs'];count=assessment['registered_views']
        events.artifact('mask_sheet',frontend/'mask-contact-sheet.png','Object masks')
        exported=None
        if args.data_root:
            args.data_root.mkdir(parents=True,exist_ok=True)
            export=args.data_root/('recovered-'+fingerprint[:16])
            if not export.exists():
                if inputs.stat().st_dev != args.data_root.stat().st_dev:
                    raise ValueError('reusable capture scenes require data and runs on the same filesystem')
                pending=export.with_name(export.name+'.pending')
                if pending.exists():shutil.rmtree(pending)
                shutil.copytree(inputs,pending,copy_function=os.link)
                pending.rename(export)
            source_hashes=inventory(inputs,['.'])
            if inventory(export,['.'])!=source_hashes:raise ValueError('recovered scene changed; keeping both copies for inspection')
            exported=str(export.relative_to(args.data_root))
            (args.output/'recovered-scene.json').write_text(json.dumps({'inputs':exported,'input_fingerprint':fingerprint,'sha256':source_hashes})+'\n')
        command=[native,'run','--inputs',inputs,'--output',args.output/'reconstruction','--base-surface',args.base_surface,'--threads','2','--minimum-free-gib',str(args.minimum_free_gib),'--no-live-previews']
        if args.gaussian:
            command+=['--splats',args.gaussian_quality]
        for item in args.gaussian_set: command+=['--splats-set',item]
        if args.texture: command+=['--texture']
        execute('reconstruction',command,native=True)
        primary=args.output/'reconstruction'/('splats/mesh.stl' if args.gaussian else 'mesh/mesh.stl')
        execute('all-view-check',[native,'check','--inputs',frontend/'inputs','--mesh',primary,'--output',args.output/'all-view-check','--check-views',str(count)],stage='check')
        events.artifact('report',args.output/'all-view-check/result.json','Photo agreement')
        if json.loads((args.output/'all-view-check/result.json').read_text())['silhouette_iou_input_masks']['median']<.8:
            raise ValueError('The mesh is inconsistent with the object masks. Review the retained mesh and photo check.')
        timings[:]=[row for row in timings if row['stage'] in used_stages]
        report={'status':'complete','seconds':time.monotonic()-start,'stages':timings,'input_photos':len(list(originals.iterdir())),'registered_views':count,'reconstruction_views':len(json.loads((inputs/'cameras.json').read_text())['views']),'base_surface':args.base_surface,'gaussian':args.gaussian,'mesh':str(primary),'reference_used':False,'manual_pixel_or_pose_tweaks':False,'resumed':args.resume,'adopted_predecessor':args.adopt_completed,'recovered_inputs':exported,'sum_successful_stage_seconds':sum(row['seconds'] for row in timings),'model_sources_bundled':False}
        report['adopted_predecessor']=any(row.get('adopted_local_predecessor',False) for row in cache.entries.values())
        report['camera_path']=json.loads((args.output/'camera-path.json').read_text())
        report['mask_agreement_gate_passed']=True
        report['imported_masks']=args.masks is not None
        report['capture_groups']=args.capture_groups
        (args.output/'pipeline.json').write_text(json.dumps(report,indent=2)+'\n')
        events.emit('run_finished',status='complete',seconds=report['seconds'])
    except BaseException as error:
        events.emit('error',message=str(error)); events.emit('run_finished',status='cancelled' if isinstance(error,Cancelled) else 'failed',seconds=time.monotonic()-start)
        raise


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['photos','output','tools']: parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--data-root',type=Path,help='Host data folder for a byte-identical linked scene; enables later refinement without camera recovery')
    parser.add_argument('--foreground-provider',choices=['app','independent'],default='app',help='App foreground tracking and recovery, or independent Vision detections')
    parser.add_argument('--camera-search',choices=['hybrid','tight','wide'],default='hybrid',help='First recovery path; the other paths remain automatic fallbacks')
    parser.add_argument('--shared-crop',action='store_true',help='Optional common frame crop before camera recovery; native per-view object crops remain automatic')
    parser.add_argument('--masks',type=Path,help='Own full-frame mask folder, e.g. saved by the app crop preview')
    parser.add_argument('--refine-imported-masks',action='store_true',help='Also run SAM on imported own masks; otherwise retain their contours')
    parser.add_argument('--refine-masks',action='store_true',default=True,help='SAM foreground refinement (default for automatic foreground)')
    parser.add_argument('--no-refine-masks',action='store_false',dest='refine_masks',help='Faster native foreground path; preserves the app foreground contours')
    parser.add_argument('--object-crop-padding',type=float,default=.15)
    parser.add_argument('--capture-groups',default='',help='Optional camera-height photo counts for records only; never an angle prior')
    parser.add_argument('--resume',action='store_true')
    parser.add_argument('--adopt-completed',action='store_true',help='Operator migration of locally generated predecessor stages; explicitly recorded, not an uninterrupted fresh run')
    parser.add_argument('--calibration',type=Path)
    parser.add_argument('--fov',type=float)
    parser.add_argument('--base-surface',choices=['silhouette','stereo'],default='silhouette')
    parser.add_argument('--gaussian',action='store_true')
    parser.add_argument('--gaussian-quality',choices=['capture','fast','standard','best'],default='capture')
    parser.add_argument('--gaussian-set',action='append',default=[])
    parser.add_argument('--texture',action='store_true')
    parser.add_argument('--minimum-free-gib',type=float,default=8.8)
    args=parser.parse_args()
    if not math.isfinite(args.minimum_free_gib) or args.minimum_free_gib<0: parser.error('invalid free-space reserve')
    if not math.isfinite(args.object_crop_padding) or not (0<=args.object_crop_padding<=1 or args.object_crop_padding==-1):parser.error('invalid object crop padding')
    if args.capture_groups and (not all(v.isdigit() and int(v)>0 for v in args.capture_groups.split(',')) or len(args.capture_groups.split(','))>16):parser.error('invalid capture group counts')
    if args.adopt_completed and not args.resume:parser.error('--adopt-completed requires --resume')
    for item in args.gaussian_set:
        key, sep, value=item.partition('=')
        if not sep or key not in {'steps','crop','mesh','seed'} or not value.isdigit(): parser.error('Gaussian overrides require steps/crop/mesh/seed=INTEGER')
    def interrupted(signum,frame): raise Cancelled('cancelled')
    signal.signal(signal.SIGTERM,interrupted)
    args.photos=args.photos.resolve();args.output=args.output.resolve();run(args)


if __name__=='__main__': main()
