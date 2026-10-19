#!/usr/bin/env python
"""Optional BSD PyCOLMAP camera path from own cached foreground matches.

Keeps native recovery available. No angle prior and no supplied poses. Refining
focal/radial terms is an explicit hypothesis, not measured lens calibration.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import time

for variable in ['OMP_NUM_THREADS', 'OPENBLAS_NUM_THREADS', 'VECLIB_MAXIMUM_THREADS']:
    os.environ.setdefault(variable, '2')
import numpy as np
if __package__:
    from .learned_matches import validated_pair
else:
    from learned_matches import validated_pair


def validate_cache(data):
    names, points, lens = data['names'], data['keypoints'], data['lens']
    if not 3 <= len(names) <= 1536 or len(set(names)) != len(names) or len(points) != len(names):
        raise ValueError('expected 3..1536 uniquely named photos/keypoint arrays')
    if any(Path(name).name != name for name in names):
        raise ValueError('photo names must be basenames')
    fx, fy, cx, cy = map(float, lens['pixels'])
    radial = list(map(float, lens['k']))
    if (not np.isfinite([fx, fy, cx, cy, *radial]).all() or min(fx, fy) <= 0
            or abs(fx - fy) > 1e-8 * fx or len(radial) != 3 or any(radial[1:])
            or min(lens['width'], lens['height']) <= 0):
        raise ValueError('this path requires finite equal-focal SIMPLE_RADIAL lens (k2=k3=0)')
    keys = []
    for values in points:
        array = np.asarray(values, dtype=float)
        if array.ndim != 2 or array.shape[1] != 2 or not np.isfinite(array).all():
            raise ValueError('finite Nx2 source keypoints required')
        keys.append(array + .5)  # engine match cache integer centres -> COLMAP half centres
    seen = set()
    pairs = []
    for pair in data['pairs']:
        a, b = pair['first'], pair['second']
        if not isinstance(a, int) or not isinstance(b, int) or not 0 <= a < b < len(names) or (a, b) in seen:
            raise ValueError('invalid/duplicate canonical photo pair')
        seen.add((a, b))
        pairs.append((a, b, validated_pair(pair['matches'], len(keys[a]), len(keys[b])).astype('uint32')))
    return [fx, cx + .5, cy + .5, radial[0]], keys, pairs


def prune_support(model, minimum):
    """Remove unsupported poses; removing tracks can expose another weak pose.

    Existing camera coordinates stay unchanged. The caller still audits the
    remaining model; this does not lower any registration/geometry threshold.
    """
    if not isinstance(minimum,int) or not 0<=minimum<=100_000:raise ValueError('invalid minimum landmark support')
    removed=[]
    while minimum:
        weak=[image_id for image_id in sorted(model.reg_image_ids()) if model.images[image_id].num_points3D<minimum]
        if not weak:break
        for image_id in weak:
            image=model.images[image_id]
            removed.append({'name':image.name,'observations':image.num_points3D})
            model.deregister_image(image_id)
    return removed


def recover(args):
    import pycolmap
    data = json.loads(args.matches.read_text())
    params, keys, pairs = validate_cache(data)
    if any(not (args.photos / name).is_file() for name in data['names']):
        raise ValueError('every original photo must exist')
    args.output.mkdir(parents=True, exist_ok=False)
    camera = pycolmap.Camera(model='SIMPLE_RADIAL', width=data['lens']['width'], height=data['lens']['height'], params=params, camera_id=1)
    unknown_focal=getattr(args,'unknown_focal',False)
    minimum_support=getattr(args,'minimum_observations',0)
    camera.has_prior_focal_length = not unknown_focal
    database = args.output / 'database.db'
    db = pycolmap.Database();db.open(str(database))
    start = time.monotonic()
    try:
        db.write_camera(camera, use_camera_id=True)
        for i, (name, keypoints) in enumerate(zip(data['names'], keys)):
            db.write_image(pycolmap.Image(name=name, camera_id=1, id=i + 1), use_image_id=True)
            db.write_keypoints(i + 1, keypoints.astype('float32'))
        options = pycolmap.TwoViewGeometryOptions();options.ransac.max_error = args.pair_error
        verified = []
        for a, b, matches in pairs:
            geometry = pycolmap.estimate_two_view_geometry(camera, keys[a], camera, keys[b], matches, options)
            db.write_matches(a + 1, b + 1, matches)
            db.write_two_view_geometry(a + 1, b + 1, geometry)
            verified.append(len(geometry.inlier_matches))
    finally:
        db.close()
    options = pycolmap.IncrementalPipelineOptions()
    options.num_threads = options.mapper.num_threads = args.threads
    options.min_model_size = 8
    options.ba_refine_focal_length = args.calibration != 'fixed'
    options.ba_refine_principal_point = False
    options.ba_refine_extra_params = args.calibration == 'radial'
    models = pycolmap.incremental_mapping(str(database), str(args.photos), str(args.output / 'models'), options=options)
    summaries = []
    for index, model in models.items():
        before=model.num_reg_images()
        removed=prune_support(model,minimum_support)
        folder = args.output / f'model-{index}';folder.mkdir()
        model.update_point_3d_errors();model.write_text(str(folder))
        summaries.append({'index': index, 'cameras': model.num_reg_images(), 'cameras_before_support_pruning':before,'removed_sparse_views':removed,'points': model.num_points3D(),
                          'mean_reprojection_original_px': model.compute_mean_reprojection_error(),
                          'intrinsics': {str(cid): cam.params.tolist() for cid, cam in model.cameras.items()}})
    # Largest connected reconstruction first: a tiny two-camera overfit can have
    # a deceptively small reprojection residual. Selection is NOT quality approval.
    best = max((row for row in summaries if row['cameras']>0 and row['points']>0), key=lambda row: (row['cameras'], row['points']), default=None)
    if best:
        f, cx, cy, k1 = next(iter(best['intrinsics'].values()))
        lens = {'schema': 'crispmesh_lens_calibration_v1', 'model': 'radialk3',
                'principal_point_convention': 'pixel_centre', 'calibration_width': data['lens']['width'],
                'calibration_height': data['lens']['height'], 'fx': f, 'fy': f, 'cx': cx - .5, 'cy': cy - .5,
                'k1': k1, 'k2': 0., 'k3': 0.}
        (args.output / 'own-estimated-lens.json').write_text(json.dumps(lens, indent=2) + '\n')
    report = {'method': 'Optional PyCOLMAP incremental mapping from own feature cache', 'quality_accepted': False,
              'reference_inputs_used': False, 'calibration_mode': args.calibration, 'has_focal_prior':not unknown_focal, 'models': summaries,
              'minimum_observations_per_view':minimum_support,'cameras_before_support_pruning':best['cameras_before_support_pruning'] if best else 0,
              'removed_sparse_views':best['removed_sparse_views'] if best else [],
              'selected_model': f"model-{best['index']}" if best else None,
              'pair_error_original_px': args.pair_error, 'threads': args.threads,
              'initial_lens': data['lens'], 'match_pixel_convention': 'integer centres; COLMAP receives +.5',
              'candidate_pairs': len(pairs), 'geometries_with15inliers': sum(n >= 15 for n in verified),
              'seconds': time.monotonic() - start, 'pycolmap': pycolmap.__version__,
              'matches_sha256': hashlib.sha256(args.matches.read_bytes()).hexdigest(),
              'limits': 'Caller must supply own-photo matches. Lens is an estimate; counts/residual do not establish shape accuracy. Inspect silhouettes and grey mesh separately.'}
    (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    if best is None:
        raise ValueError('no connected model recovered; report retained')
    print(json.dumps(report, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['matches', 'photos', 'output']:
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--calibration', choices=['fixed', 'focal', 'radial'], default='fixed')
    parser.add_argument('--threads', type=int, default=2)
    parser.add_argument('--pair-error', type=float, default=3.)
    parser.add_argument('--unknown-focal',action='store_true',help='Treat the initial focal length as a hypothesis, not measured calibration; retains the calibrated-prior path by default')
    parser.add_argument('--minimum-observations',type=int,default=0,help='Drop poses with fewer distinct landmarks, iterating after track removal (default: retain the original model)')
    args = parser.parse_args()
    if not 1 <= args.threads <= 16 or not np.isfinite(args.pair_error) or args.pair_error <= 0 or not 0<=args.minimum_observations<=100_000:
        parser.error('invalid threads/pair error')
    recover(args)


if __name__ == '__main__':
    main()
