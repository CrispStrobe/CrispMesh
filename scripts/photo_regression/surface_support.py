"""Audit a mesh against its own depths, with no independent reference inputs.

This is an evidence-support diagnostic, not a geometry accuracy score. Mesh
samples come after extrapolation/smoothing; direct support at them is not the
support at fused voxel centres. Mask-rim erosion is not reproduced, so counted
observations are an upper bound on production fusion support.
"""
import argparse
import json
from pathlib import Path
import numpy as np

try:
    from .detail_profiles import read_stl
except ImportError:
    from detail_profiles import read_stl


def sample_surface(triangles, count, seed):
    triangles = np.asarray(triangles, dtype=float)
    if triangles.ndim != 3 or triangles.shape[1:] != (3, 3) or not np.isfinite(triangles).all():
        raise ValueError('mesh triangles must be finite N x 3 x 3')
    area = np.linalg.norm(np.cross(triangles[:, 1]-triangles[:, 0], triangles[:, 2]-triangles[:, 0]), axis=1)
    if count < 1 or not np.isfinite(area.sum()) or area.sum() <= 0:
        raise ValueError('positive sample count and nonzero mesh area required')
    rng = np.random.default_rng(seed)
    picked = triangles[rng.choice(len(triangles), count, p=area/area.sum())]
    u, v = rng.random((2, count));u = np.sqrt(u)
    return (1-u[:, None])*picked[:, 0]+(u*(1-v))[:, None]*picked[:, 1]+(u*v)[:, None]*picked[:, 2]


def support_counts(points, rows, stage_rows, depths, truncation, behind_distance):
    points = np.asarray(points, dtype=float)
    if points.ndim != 2 or points.shape[1] != 3 or not np.isfinite(points).all():
        raise ValueError('surface points must be finite N x 3')
    if not (0 < truncation <= behind_distance) or not np.isfinite(behind_distance):
        raise ValueError('require finite 0 < truncation <= behind distance')
    if not rows or len(rows) != len(stage_rows) or len(rows) != len(depths):
        raise ValueError('camera, stage camera and depth counts must match and be nonzero')
    counts = {name: np.zeros(len(points), dtype=np.int32) for name in ['near', 'free', 'behind', 'positive', 'negative']}
    for row, stage, depth in zip(rows, stage_rows, depths):
        r = np.asarray(row['rotation'], float);t = np.asarray(row['translation'], float);k = np.asarray(stage['k'], float)
        if r.shape != (3, 3) or t.shape != (3,) or k.shape != (4,) or not np.isfinite(np.concatenate((r.ravel(), t, k))).all() or (k[:2] <= 0).any():
            raise ValueError('invalid camera parameters')
        depth = np.asarray(depth)
        if depth.shape != (stage['height'], stage['width']) or not np.isfinite(depth).all() or (depth < 0).any():
            raise ValueError('depth must be finite, nonnegative and match stage dimensions')
        pc = points@r.T+t;z = pc[:, 2]
        uv = pc[:, :2]/np.maximum(z[:, None], 1e-12)*k[:2]+k[2:]-.5
        # Reject before converting: points behind the camera must not overflow
        # integer pixel indices, and clipping must not turn them into evidence.
        h, w = depth.shape
        inside = (z > 0) & (uv[:, 0] >= -.5) & (uv[:, 0] <= w-.5) & (uv[:, 1] >= -.5) & (uv[:, 1] <= h-.5)
        measured = np.zeros(len(points))
        at = np.flatnonzero(inside);ix, iy = np.rint(uv[at]).astype(np.int64).T
        valid = (ix >= 0) & (ix < w) & (iy >= 0) & (iy < h)
        measured[at[valid]] = depth[iy[valid], ix[valid]]
        seen = measured > 0;sdf = measured-z;near = seen & (np.abs(sdf) < truncation)
        counts['near'] += near
        counts['positive'] += near & (sdf >= 0)
        counts['negative'] += near & (sdf < 0)
        counts['free'] += seen & (sdf >= truncation)
        counts['behind'] += seen & (sdf <= -truncation) & (sdf > -behind_distance)
    return counts


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mesh', type=Path, required=True)
    parser.add_argument('--cameras', type=Path, required=True)
    parser.add_argument('--stage-cameras', type=Path, required=True, help='CRISPMESH_STAGE_DEPTHS stage camera metadata with cropped intrinsics')
    parser.add_argument('--depths', type=Path, required=True)
    parser.add_argument('--truncation', type=float, required=True, help='scene units; same truncation used for fusion')
    parser.add_argument('--behind-distance', type=float, required=True, help='scene units; fusion behind band')
    parser.add_argument('--samples', type=int, default=100000)
    parser.add_argument('--seed', type=int, default=236)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    try:
        points = sample_surface(read_stl(args.mesh), args.samples, args.seed)
        rows = json.loads(args.cameras.read_text())['views'];stage = json.loads(args.stage_cameras.read_text())['views']
        with np.load(args.depths, allow_pickle=False) as archive:
            counts = support_counts(points, rows, stage, [archive[f'depth_{i:03}'] for i in range(len(rows))], args.truncation, args.behind_distance)
        near = counts['near']
        report = {
            'schema': 'crispmesh_surface_support_v1', 'reference_used': False,
            'samples': args.samples, 'seed': args.seed, 'truncation': args.truncation, 'behind_distance': args.behind_distance,
            'near_surface_views_histogram': {str(i): int((near == i).sum()) for i in range(int(near.max())+1)},
            'fraction_near_surface_at_least': {str(i): float(np.mean(near >= i)) for i in [1, 2, 3, 4, 5]},
            'zero_near_but_has_free_space': float(np.mean((near == 0) & (counts['free'] > 0))),
            'zero_near_but_has_behind_votes': float(np.mean((near == 0) & (counts['behind'] > 0))),
            'fraction_near_support_on_both_sides': float(np.mean((counts['positive'] > 0) & (counts['negative'] > 0))),
            'rim_filter_applied': False,
            'limits': ['After smoothing/extrapolation, not exact fused voxel centres', 'No mask-rim erosion: counted support is an upper bound', 'Near-depth support does not establish accuracy; no independent reference is read'],
        }
        args.output.write_text(json.dumps(report, indent=2)+'\n')
    except (ValueError, KeyError, OSError) as error:
        parser.error(str(error))


if __name__ == '__main__':
    main()
