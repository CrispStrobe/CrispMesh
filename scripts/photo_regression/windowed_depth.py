#!/usr/bin/env python
"""Experimental bounded CPU DA3-BASE depth from OWN recovered pinhole inputs.

No camera-recovery or quality-acceptance claim. Uses the pinned Apache core,
not its API/evo dependency. Retains source camera centres/world units; virtual
re-framing is a pinhole rotation, not an ordinary off-centre crop.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import threading
import time
import zipfile

for variable in ['OMP_NUM_THREADS', 'OPENBLAS_NUM_THREADS', 'VECLIB_MAXIMUM_THREADS']:
    os.environ.setdefault(variable, '2')

import numpy as np
from PIL import Image
from scipy.ndimage import map_coordinates
from scipy.spatial.transform import Rotation

SOURCE_REVISION = '3d835ec1a5802d64a8b8b15f817a1ab54809bfe4'
CHECKPOINT_HASH = 'e01067dc1659613083d9145a9a2547ccdbe6ccbbf83c4fe7b3e8a4e2bdae78b5'
CONFIG_HASH = '5e34115ebc17bd2d8d43033c5f72e9446ac8833fd61d3fa160b7e67e0bb5b7b5'


def digest(path):
    result = hashlib.sha256()
    with Path(path).open('rb') as stream:
        while data := stream.read(4 << 20):
            result.update(data)
    return result.hexdigest()


def atomic_json(path, value):
    pending = path.with_suffix('.pending')
    pending.write_text(json.dumps(value, indent=2) + '\n')
    pending.replace(path)


def atomic_array(path, array):
    pending = path.with_suffix('.pending')
    with pending.open('wb') as stream:
        np.save(stream, array, allow_pickle=False)
    pending.replace(path)


def virtual_camera(mask, half_k, size, padding=.15):
    ys, xs = np.where(np.asarray(mask) > 127)
    if not len(xs):
        raise ValueError('empty foreground')
    fx, fy, cx, cy = np.asarray(half_k, dtype=float)
    if not np.isfinite([fx, fy, cx, cy]).all() or min(fx, fy) <= 0:
        raise ValueError('finite pinhole intrinsics required')
    k = np.array([[fx, 0, cx - .5], [0, fy, cy - .5], [0, 0, 1.]])
    middle = [(xs.min() + xs.max()) / 2, (ys.min() + ys.max()) / 2, 1.]
    forward = np.linalg.solve(k, middle)
    forward /= np.linalg.norm(forward)
    right = np.cross([0., 1., 0.], forward)
    right /= np.linalg.norm(right)
    rotation = np.stack([right, np.cross(forward, right), forward])
    corners = np.array([[xs.min() - .5, ys.min() - .5, 1], [xs.max() + .5, ys.min() - .5, 1],
                        [xs.min() - .5, ys.max() + .5, 1], [xs.max() + .5, ys.max() + .5, 1]]).T
    rays = rotation @ np.linalg.solve(k, corners)
    if np.any(rays[2] <= 0):
        raise ValueError('foreground crosses virtual horizon')
    extent = np.abs(rays[:2] / rays[2]).max()
    focal = (size / 2 - 1) / ((1 + 2 * padding) * extent)
    target = np.array([[focal, 0, size / 2], [0, focal, size / 2], [0, 0, 1.]])
    return target @ rotation @ np.linalg.inv(k), rotation, target


def align_points(model_poses, rows, points):
    """Match two versions of the SAME own recovery; refuse different camera fits."""
    shared = [row for row in rows if row['source'] in model_poses]
    if len(shared) < 3:
        raise ValueError('at least three matching own camera names required')
    first = np.array([-r.T @ t for r, t in [model_poses[v['source']] for v in shared]])
    second = np.array([-np.asarray(v['rotation']).T @ v['translation'] for v in shared])
    x, y = first - first.mean(0), second - second.mean(0)
    u, singular, vt = np.linalg.svd(x.T @ y)
    if singular[1] <= singular[0] * 1e-8:
        raise ValueError('camera centres do not constrain a world similarity')
    signs = np.ones(3);signs[-1] = np.sign(np.linalg.det(u @ vt))
    row_rotation = u @ np.diag(signs) @ vt
    scale = float(np.sum(singular * signs) / np.sum(x * x))
    offset = second.mean(0) - scale * first.mean(0) @ row_rotation
    span = max(float(np.ptp(second, axis=0).max()), 1e-9)
    error = np.linalg.norm(scale * first @ row_rotation + offset - second, axis=1).max() / span
    rotation_error = max(np.abs(np.asarray(v['rotation']) - model_poses[v['source']][0] @ row_rotation).max() for v in shared)
    if scale <= 0 or error > 1e-5 or rotation_error > 1e-5:
        raise ValueError('landmarks and inputs are not the same camera recovery/frame')
    return scale * np.asarray(points) @ row_rotation + offset


def original_sources(rows, photo_map=None):
    """Resolve production staging names explicitly, never by list position."""
    if len({row['source'] for row in rows}) != len(rows):
        raise ValueError('duplicate input source names')
    if photo_map is None:
        return rows
    entries = photo_map['photos']
    captures = [entry['capture'] for entry in entries]
    sources = [entry['source'] for entry in entries]
    if len(set(captures)) != len(captures) or len(set(sources)) != len(sources):
        raise ValueError('photo map must be one-to-one')
    mapping = dict(zip(captures, sources))
    if any(row['source'] not in mapping for row in rows):
        raise ValueError('photo map does not cover input sources')
    return [dict(row, input_source=row['source'], source=mapping[row['source']]) for row in rows]


def read_own_landmarks(folder, rows):
    poses, ids = {}, {}
    lines = iter((folder / 'images.txt').read_text().splitlines())
    for line in lines:
        if not line.strip() or line.startswith('#'):
            continue
        parts = line.split(maxsplit=9)
        if len(parts) != 10:
            raise ValueError('invalid own COLMAP image record')
        image_id, name = int(parts[0]), parts[9]
        q = np.array(parts[1:5], dtype=float)
        rotation = Rotation.from_quat([*q[1:], q[0]]).as_matrix()
        poses[name] = rotation, np.array(parts[5:8], dtype=float)
        ids[image_id] = name
        next(lines, '')  # consume this image's POINTS2D line, including empty lines
    points, tracks = [], []
    for line in (folder / 'points3D.txt').read_text().splitlines():
        if not line.strip() or line.startswith('#'):
            continue
        p = line.split()
        points.append(list(map(float, p[1:4])))
        tracks.append([ids[int(i)] for i in p[8::2]])
    points = np.asarray(points, dtype=float)
    if points.ndim != 2 or points.shape[1] != 3 or not len(points) or not np.isfinite(points).all():
        raise ValueError('finite nonempty own 3D landmarks required')
    if any(row['source'] not in poses for row in rows):
        raise ValueError('own model must cover every input source')
    points = align_points(poses, rows, points)
    observed = {row['source']: [] for row in rows}
    for index, names in enumerate(tracks):
        for name in names:
            if name in observed:
                observed[name].append(index)
    return {name: points[indices] for name, indices in observed.items()}


def anchor_depth(depth, mask, rotation, translation, k, landmarks, minimum=20):
    camera = np.asarray(landmarks) @ np.asarray(rotation).T + translation
    z = camera[:, 2]
    uv = camera[:, :2] / np.maximum(z[:, None], 1e-12) * [k[0, 0], k[1, 1]] + [k[0, 2], k[1, 2]]
    h, w = depth.shape
    valid = (z > 0) & np.isfinite(uv).all(1) & (uv[:, 0] >= 0) & (uv[:, 0] < w - 1) & (uv[:, 1] >= 0) & (uv[:, 1] < h - 1)
    samples = map_coordinates(depth, [uv[:, 1], uv[:, 0]], order=1, mode='constant', cval=0)
    foreground = map_coordinates(mask.astype(float), [uv[:, 1], uv[:, 0]], order=1, mode='constant', cval=0) > .999
    valid &= foreground & np.isfinite(samples) & (samples > 0)
    indices = np.flatnonzero(valid)
    train, held = indices[indices % 5 != 0], indices[indices % 5 == 0]
    if len(train) < minimum:
        return None, {'status': 'insufficient_observed_scale_landmarks', 'fit_points': len(train), 'heldout_points': len(held)}
    ratios = z[train] / samples[train]
    median = float(np.median(ratios));mad = float(np.median(np.abs(ratios - median)))
    kept = np.abs(ratios - median) <= max(3.5 * 1.4826 * mad, .01 * median)
    scale = float(np.median(ratios[kept]))
    if not np.isfinite(scale) or scale <= 0:
        raise ValueError('invalid sparse depth scale')
    error = np.abs(samples[held] * scale - z[held]) / z[held]
    report = {'status': 'scaled', 'scale': scale, 'fit_points': len(train), 'retained_fit_points': int(kept.sum()),
              'heldout_points': len(held), 'heldout_median_relative_z_error': float(np.median(error)) if len(error) else None,
              'limits': 'Held out from scale fitting only; own SfM is uncertain, not independent geometry/camera validation.'}
    return np.where(mask, depth * scale, 0).astype('float32'), report


def reframe_depth(depth, source_k, target_k, width, height):
    yy, xx = np.indices((height, width))
    a, b = np.asarray(source_k), np.asarray(target_k)
    x = (xx + .5 - b[2]) / b[0] * a[0] + a[2] - .5
    y = (yy + .5 - b[3]) / b[1] * a[1] + a[3] - .5
    valid = map_coordinates((depth > 0).astype('float32'), [y, x], order=1, mode='constant', cval=0) >= .999999
    result = map_coordinates(depth, [y, x], order=1, mode='constant', cval=0)
    return np.where(valid, result, 0).astype('float32')


def prepare_view(inputs, row, output, size):
    import cv2
    cv2.setNumThreads(2)
    with Image.open(inputs / row.get('texture_image', row['image'])) as image, Image.open(inputs / row['mask']) as alpha:
        rgb, mask = np.array(image.convert('RGB')), np.array(alpha.convert('L'))
    if rgb.shape[:2] != mask.shape or mask.shape != (row['height'], row['width']):
        raise ValueError('input image/mask/camera dimensions disagree')
    homography, q, k = virtual_camera(mask, row['k'], size)
    up = np.array([[2., 0., .5], [0., 2., .5], [0., 0., 1.]])
    rgb = cv2.warpPerspective(rgb, up @ homography, (size * 2, size * 2), flags=cv2.INTER_LINEAR, borderValue=(255, 255, 255))
    rgb = cv2.resize(rgb, (size, size), interpolation=cv2.INTER_AREA)
    mask = cv2.warpPerspective(mask, homography, (size, size), flags=cv2.INTER_NEAREST, borderValue=0) > 127
    rgb[~mask] = 255
    for name, array in [('images', rgb), ('masks', mask.astype('uint8') * 255)]:
        Image.fromarray(array).save(output / name / (row['name'] + '.png'))
    return {'name': row['name'], 'source': row['source'], 'image': f"images/{row['name']}.png", 'mask': f"masks/{row['name']}.png",
            'width': size, 'height': size, 'k': [k[0, 0], k[1, 1], k[0, 2] + .5, k[1, 2] + .5],
            'rotation': (q @ row['rotation']).tolist(), 'translation': (q @ row['translation']).tolist(),
            'input_source': row.get('input_source', row['source']),
            'virtual_camera': {'original_integer_to_virtual_integer': homography.tolist(), 'rotation': q.tolist()}}


def window_for(reference, remaining, directions, count):
    angles = np.degrees(np.arccos(np.clip(directions @ directions[reference], -1, 1)))
    candidates = [i for i in range(len(directions)) if i != reference and 6 <= angles[i] <= 60]
    if len(candidates) < count - 1:
        candidates = [i for i in range(len(directions)) if i != reference and angles[i] > .001]
    candidates.sort(key=lambda i: (i not in remaining, abs(angles[i] - 20), i))
    if len(candidates) < 1:
        raise ValueError('no distinct camera directions for a conditioned window')
    return [reference] + candidates[:count - 1]


def run_inference(args):
    if digest(args.checkpoint) != CHECKPOINT_HASH or digest(args.config) != CONFIG_HASH:
        raise ValueError('expected pinned Apache DA3-BASE checkpoint/config')
    revision = subprocess.check_output(['git', '-C', str(args.source), 'rev-parse', 'HEAD'], text=True).strip()
    if revision != SOURCE_REVISION:
        raise ValueError('unexpected official core revision')
    subprocess.run(['git', '-C', str(args.source), 'diff', '--exit-code', '--', 'src/depth_anything_3'], check=True, stdout=subprocess.DEVNULL)
    import resource
    def watch():
        while True:
            peak = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss * (1 if sys.platform == 'darwin' else 1024)
            if peak > args.maximum_rss_gib * 2**30:
                print('CPU depth probe exceeded peak RSS guard', flush=True);os._exit(86)
            time.sleep(.1)
    threading.Thread(target=watch, daemon=True).start()
    rows = original_sources(json.loads((args.inputs / 'cameras.json').read_text())['views'],
                            json.loads(args.photo_map.read_text()) if args.photo_map else None)
    if not 3 <= len(rows) <= 1536 or len({v['name'] for v in rows}) != len(rows):
        raise ValueError('expected 3..1536 uniquely named views')
    if any(Path(v['name']).name != v['name'] for v in rows):
        raise ValueError('view names must be basenames')
    observed = read_own_landmarks(args.sparse_model, rows)
    provenance = {'inputs_cameras_sha256': digest(args.inputs / 'cameras.json'),
                  'inputs_sparse_points_sha256': digest(args.inputs / 'sparse_points.npy'),
                  'own_model_images_sha256': digest(args.sparse_model / 'images.txt'),
                  'own_model_points_sha256': digest(args.sparse_model / 'points3D.txt'),
                  'source_files': [{'source': row['source'], 'rgb_sha256': digest(args.inputs / row.get('texture_image', row['image'])),
                                    'mask_sha256': digest(args.inputs / row['mask'])} for row in rows],
                  'photo_map_sha256': digest(args.photo_map) if args.photo_map else None,
                  'reference_inputs_used': False, 'limits': 'Caller must establish own-photo provenance; hashes alone cannot establish it.'}
    args.output.mkdir(parents=True, exist_ok=False)
    atomic_json(args.output / 'provenance.json', provenance)
    inputs = args.output / 'inputs'
    for folder in ['images', 'masks']:
        (inputs / folder).mkdir(parents=True, exist_ok=True)
    (args.output / 'depths').mkdir()
    prepared = [prepare_view(args.inputs, row, inputs, args.resolution) for row in rows]
    atomic_json(inputs / 'cameras.json', {'views': prepared})
    atomic_array(inputs / 'sparse_points.npy', np.load(args.inputs / 'sparse_points.npy', allow_pickle=False))
    os.environ.setdefault('OMP_NUM_THREADS', '2')
    sys.path.insert(0, str(args.source / 'src'))
    import torch
    from omegaconf import OmegaConf
    from safetensors.torch import load_model
    from depth_anything_3.cfg import create_object
    torch.set_num_threads(2);torch.set_num_interop_threads(2)
    model = create_object(OmegaConf.create(json.loads(args.config.read_text())['config'])).eval().float()
    wrapper = torch.nn.Module();wrapper.add_module('model', model)
    load_model(wrapper, args.checkpoint, strict=True, device='cpu')  # handles shared auxiliary-head aliases
    centres = np.array([-np.asarray(v['rotation']).T @ v['translation'] for v in prepared])
    object_centre = np.median(np.load(inputs / 'sparse_points.npy', allow_pickle=False), axis=0)
    directions = centres - object_centre
    directions /= np.linalg.norm(directions, axis=1, keepdims=True)
    remaining, reports, windows = set(range(len(rows))), {}, []
    start = time.monotonic()
    while remaining:
        group = window_for(min(remaining), remaining, directions, args.window)
        rgb, masks, ext, intr = [], [], [], []
        for i in group:
            row = prepared[i]
            with Image.open(inputs / row['image']) as image, Image.open(inputs / row['mask']) as mask:
                rgb.append(np.array(image));masks.append(np.array(mask) > 127)
            e = np.eye(4);e[:3, :3] = row['rotation'];e[:3, 3] = row['translation'];ext.append(e)
            fx, fy, cx, cy = row['k'];intr.append([[fx, 0, cx - .5], [0, fy, cy - .5], [0, 0, 1.]])
        # Same camera normalization convention as the pinned Apache core.
        ext = np.stack(ext);normalized = ext @ np.linalg.inv(ext[0])
        distance = np.linalg.norm(np.linalg.inv(normalized)[:, :3, 3], axis=1)
        baseline = max(float(np.sort(distance)[(len(distance) - 1) // 2]), .1)
        normalized[:, :3, 3] /= baseline
        image = torch.from_numpy(np.stack(rgb)).permute(0, 3, 1, 2).float() / 255
        image = ((image - torch.tensor([.485, .456, .406])[None, :, None, None]) / torch.tensor([.229, .224, .225])[None, :, None, None])[None].contiguous()
        with torch.inference_mode():
            prediction = model(image, extrinsics=torch.from_numpy(normalized[None].astype('float32')),
                               intrinsics=torch.tensor(intr, dtype=torch.float32)[None], ref_view_strategy='first')
        depth = prediction.depth[0].numpy()
        if not np.isfinite(depth).all() or np.any(depth <= 0):
            raise ValueError('nonfinite or nonpositive learned depth')
        for local, i in enumerate(group):
            if i not in remaining:
                continue
            row = prepared[i]
            z, report = anchor_depth(depth[local], masks[local], row['rotation'], row['translation'], np.asarray(intr[local]), observed[row['source']], args.minimum_scale_points)
            if z is None:
                z = np.zeros_like(depth[local], dtype='float32')
            atomic_array(args.output / 'depths' / f'depth_{i:03}.npy', z)
            reports[row['source']] = report
            remaining.remove(i)
        windows.append(group)
        del prediction, image, depth
        atomic_json(args.output / 'progress.json', {'completed': len(rows) - len(remaining), 'input': len(rows), 'windows': windows, 'scale_reports': reports})
        print('conditioned depths', len(rows) - len(remaining), '/', len(rows), flush=True)
    assert not any(name == 'evo' or name.startswith('evo.') for name in sys.modules)
    atomic_json(args.output / 'report.json', {'status': 'inference_complete', 'quality_accepted': False, 'views': len(rows), 'windows': windows,
        'seconds': time.monotonic() - start, 'resolution': args.resolution, 'device': 'CPU F32, no AMP', 'camera_refinement': False,
        'reference_inputs_used': False, 'source_revision': revision, 'checkpoint_sha256': CHECKPOINT_HASH, 'config_sha256': CONFIG_HASH,
        'scale_reports': reports, 'peak_rss_bytes': resource.getrusage(resource.RUSAGE_SELF).ru_maxrss * (1 if sys.platform == 'darwin' else 1024),
        'core_module_sha256': {name: digest(module.__file__) for name, module in sys.modules.items() if name.startswith('depth_anything_3') and getattr(module, '__file__', None)},
        'api_imported': 'depth_anything_3.api' in sys.modules, 'evo_imported': False,
        'limits': 'Own SfM poses/landmarks only; sparse depth-scale diagnostics are not independent geometry validation. Neural detail may be invented or lost. Input bound1536 is not a runtime benchmark.'})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    infer = sub.add_parser('infer')
    for name in ['inputs', 'sparse-model', 'source', 'checkpoint', 'config', 'output']:
        infer.add_argument('--' + name, type=Path, required=True)
    infer.add_argument('--photo-map', type=Path, help='Production photos/photo-map.json for staged source names')
    infer.add_argument('--resolution', type=int, default=336)
    infer.add_argument('--window', type=int, default=4)
    infer.add_argument('--minimum-scale-points', type=int, default=20)
    infer.add_argument('--maximum-rss-gib', type=float, default=4.)
    reframe = sub.add_parser('reframe')
    for name in ['inputs', 'depths', 'level-report', 'output']:
        reframe.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    if args.command == 'infer':
        if not 112 <= args.resolution <= 504 or args.resolution % 14 or not 2 <= args.window <= 8 or args.minimum_scale_points < 3 or not np.isfinite(args.maximum_rss_gib) or args.maximum_rss_gib <= 0:
            parser.error('invalid resolution/window/scale/RSS budget')
        run_inference(args)
    else:
        if args.output.exists():
            parser.error('output must be fresh')
        rows = json.loads((args.inputs / 'cameras.json').read_text())['views']
        level = json.loads(args.level_report.read_text())['level_checks'][-1]
        if len(level['k']) != len(rows):
            parser.error('level camera count differs')
        width, height = level['working_size']
        pending = args.output.with_suffix('.pending')
        with zipfile.ZipFile(pending, 'w', compression=zipfile.ZIP_DEFLATED) as archive:
            for i, row in enumerate(rows):
                depth = np.load(args.depths / f'depth_{i:03}.npy', allow_pickle=False)
                if depth.shape != (row['height'], row['width']) or not np.isfinite(depth).all() or np.any(depth < 0):
                    parser.error('invalid source depth plane')
                result = reframe_depth(depth, row['k'], level['k'][i], width, height)
                with archive.open(f'depth_{i:03}.npy', 'w') as stream:
                    np.lib.format.write_array(stream, result, allow_pickle=False)
        pending.replace(args.output)


if __name__ == '__main__':
    main()
