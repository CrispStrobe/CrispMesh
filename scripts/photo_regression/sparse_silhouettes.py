#!/usr/bin/env python
"""Project object-only recovered sparse points into all own foreground masks.

A necessary silhouette-consistency diagnostic, not geometric ground truth. Do not
use background/support landmarks or dataset-supplied masks as pipeline inputs.
PyCOLMAP is only needed for CLI model loading; the arithmetic checks use NumPy.
"""
import argparse
import hashlib
import json
from pathlib import Path

import numpy as np


def projection_support(points, transform, project, mask, pixel_scale=1.):
    """COLMAP raster-corner coordinates -> original mask pixel centres.

    Every point stays in the denominator: behind-camera, nonfinite and off-frame
    points cannot count as foreground support. No occlusion or shape-accuracy
    inference is made from support alone.
    """
    if not np.isfinite(pixel_scale) or pixel_scale <= 0:
        raise ValueError('pixel scale must be positive and finite')
    points = np.asarray(points, dtype=np.float64)
    mask = np.asarray(mask, dtype=bool)
    if points.ndim != 2 or points.shape[1] != 3 or not len(points) or not np.isfinite(points).all():
        raise ValueError('expected nonempty finite Nx3 object points')
    if mask.ndim != 2 or not mask.size:
        raise ValueError('expected nonempty 2D mask')
    q = np.asarray(transform(points), dtype=np.float64)
    if q.shape != points.shape:
        raise ValueError('camera transform changed the point-array shape')
    visible = np.isfinite(q).all(axis=1) & (q[:, 2] > 0)
    uv = np.asarray(project(q[visible]), dtype=np.float64).reshape(-1, 2) if visible.any() else np.empty((0, 2))
    if len(uv) != int(visible.sum()):
        raise ValueError('camera projection changed the point count')
    uv = uv * pixel_scale - .5
    finite = np.isfinite(uv).all(axis=1)
    # Bound in float before casting; very distant projections can overflow ints.
    inside = finite & (uv[:, 0] >= -.5) & (uv[:, 0] < mask.shape[1] - .5)
    inside &= (uv[:, 1] >= -.5) & (uv[:, 1] < mask.shape[0] - .5)
    ids = np.flatnonzero(inside)
    xy = np.floor(uv[ids] + .5).astype(np.int64)
    inside[ids] &= mask[xy[:, 1], xy[:, 0]]
    return {'inside_fraction': float(inside.sum() / len(points)),
            'behind_or_nonfinite_camera_fraction': float((~visible).mean()),
            'nonfinite_projection_count': int((~finite).sum())}


def validate_dimensions(mask_shape, camera_width, camera_height, pixel_scale):
    expected = np.array([camera_height, camera_width], dtype=np.float64) * pixel_scale
    if not np.isfinite(pixel_scale) or pixel_scale <= 0 or not np.allclose(expected, mask_shape, atol=1e-6, rtol=0):
        raise ValueError(f'mask dimensions {tuple(mask_shape)} disagree with scaled camera dimensions {tuple(expected)}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--model', type=Path, required=True)
    parser.add_argument('--masks', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--pixel-scale', type=float, default=1., help='Only for a deliberately scaled diagnostic camera model')
    parser.add_argument('--maximum-points', type=int, default=20000)
    parser.add_argument('--minimum-median-support', type=float, help='Optional caller-selected rejection gate, not an accuracy guarantee')
    args = parser.parse_args()
    if args.maximum_points < 1:
        parser.error('--maximum-points must be positive')
    if args.minimum_median_support is not None and not 0 <= args.minimum_median_support <= 1:
        parser.error('--minimum-median-support must be between 0 and 1')
    import pycolmap
    from PIL import Image

    model = pycolmap.Reconstruction(str(args.model))
    ordered = sorted(model.points3D.items())
    if not ordered or not model.images:
        parser.error('model must contain registered cameras and object landmarks')
    selected = np.linspace(0, len(ordered) - 1, min(len(ordered), args.maximum_points)).round().astype(int)
    point_ids = np.array([ordered[i][0] for i in selected], dtype='<u8')
    points = np.array([ordered[i][1].xyz for i in selected])
    per_view = []
    for _, image in sorted(model.images.items()):
        camera = model.cameras[image.camera_id]
        candidates = [args.masks / (image.name + '.png'), args.masks / Path(image.name).with_suffix('.png'), args.masks / image.name]
        path = next((p for p in candidates if p.is_file()), None)
        if path is None:
            parser.error(f'no foreground mask for {image.name}')
        with Image.open(path) as source_mask:
            mask = np.array(source_mask.convert('L')) > 127
        validate_dimensions(mask.shape, camera.width, camera.height, args.pixel_scale)
        transform = image.cam_from_world
        if callable(transform):
            transform = transform()
        result = projection_support(points, lambda p: transform * p, camera.img_from_cam, mask, args.pixel_scale)
        per_view.append({'photo': image.name, 'mask_sha256': hashlib.sha256(path.read_bytes()).hexdigest(), **result})
    fractions = [v['inside_fraction'] for v in per_view]
    median = float(np.median(fractions))
    accepted = None if args.minimum_median_support is None else median >= args.minimum_median_support
    report = {'registered_views': len(per_view), 'model_points': len(ordered), 'sampled_points': len(points),
              'point_ids_sha256': hashlib.sha256(point_ids.tobytes()).hexdigest(), 'pixel_scale': args.pixel_scale,
              'median_inside_fraction': median, 'worst_inside_fraction': min(fractions),
              'requested_minimum_median_support': args.minimum_median_support, 'passes_requested_threshold': accepted,
              'per_view': per_view, 'note': 'Own object-only sparse points and own masks required. Necessary consistency only; '
              'different landmark/view sets are not paired accuracy scores. High support does not prove correct cameras, mesh or surface detail.'}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({k: v for k, v in report.items() if k != 'per_view'}, indent=2))
    return 2 if accepted is False else 0


if __name__ == '__main__':
    raise SystemExit(main())
