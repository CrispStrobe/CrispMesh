#!/usr/bin/env python
"""Optional view selection from own sparse/silhouette consistency diagnostics.

Retains camera poses/world frame and full-batch sparse bounds. This is not a
reconstruction from the selected photos alone, or a shape-accuracy guarantee.
Evaluate the mesh against all original views, including those excluded here.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil

import numpy as np


def selection(rows, sparse, audit, keep, photo_map=None, maximum_gap=20.):
    if not 8 <= keep < len(rows):
        raise ValueError('keep must retain at least eight and exclude at least one view')
    if not np.isfinite(maximum_gap) or not 0 < maximum_gap < 180:
        raise ValueError('direction coverage bound must be finite and in (0,180) degrees')
    names = [row['source'] for row in rows]
    if len(set(names)) != len(names):
        raise ValueError('input source names must be unique')
    if photo_map is not None:
        entries = photo_map['photos']
        mapping = {row['capture']: row['source'] for row in entries}
        if len(mapping) != len(entries) or len({row['source'] for row in entries}) != len(entries) or any(name not in mapping for name in names):
            raise ValueError('photo map must be one-to-one and cover every input source')
        names = [mapping[name] for name in names]
    scores = {row['photo']: row['inside_fraction'] for row in audit['per_view']}
    if len(scores) != len(audit['per_view']) or any(name not in scores for name in names):
        raise ValueError('support report must cover each original photo exactly once')
    quality = np.array([scores[name] for name in names], dtype=float)
    if not np.isfinite(quality).all() or np.any((quality < 0) | (quality > 1)):
        raise ValueError('invalid consistency fractions')
    sparse = np.asarray(sparse, dtype=float)
    if sparse.ndim != 2 or sparse.shape[1] != 3 or not len(sparse) or not np.isfinite(sparse).all():
        raise ValueError('finite own Nx3 sparse bounds required')
    rotations = np.array([row['rotation'] for row in rows]); translations = np.array([row['translation'] for row in rows])
    if (rotations.shape != (len(rows), 3, 3) or translations.shape != (len(rows), 3) or not np.isfinite(rotations).all()
            or not np.isfinite(translations).all() or np.max(np.abs(rotations @ rotations.transpose(0, 2, 1) - np.eye(3))) > 1e-5
            or np.max(np.abs(np.linalg.det(rotations) - 1)) > 1e-5):
        raise ValueError('finite rigid own cameras required')
    centres = -np.einsum('nji,nj->ni', rotations, translations)
    directions = centres - np.median(sparse, axis=0)
    lengths = np.linalg.norm(directions, axis=1)
    if np.any(lengths <= 1e-12):
        raise ValueError('camera direction undefined at sparse centroid')
    directions /= lengths[:, None]
    ranked = sorted(range(len(rows)), key=lambda i: (-quality[i], i))
    selected = sorted(ranked[:keep]); excluded = sorted(ranked[keep:])
    gaps = np.degrees(np.arccos(np.clip(directions[excluded] @ directions[selected].T, -1, 1))).min(1)
    if float(gaps.max()) > maximum_gap:
        raise ValueError(f'selection loses direction coverage: {gaps.max():.3f} degrees exceeds {maximum_gap}')
    report = {'input': len(rows), 'kept': keep, 'minimum_kept_support': float(quality[selected].min()),
              'maximum_excluded_direction_to_kept_degrees': float(gaps.max()),
              'median_excluded_direction_to_kept_degrees': float(np.median(gaps)),
              'selected_sources': [names[i] for i in selected],
              'excluded_sources': [{'photo': names[i], 'support': float(quality[i])} for i in excluded], 'quality_accepted': False}
    return selected, report


def sha(path):
    result = hashlib.sha256()
    with Path(path).open('rb') as stream:
        while data := stream.read(4 << 20): result.update(data)
    return result.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['inputs', 'support-report', 'output']:
        parser.add_argument('--'+name, type=Path, required=True)
    parser.add_argument('--photo-map', type=Path)
    parser.add_argument('--keep', type=int, required=True)
    parser.add_argument('--maximum-direction-gap', type=float, default=20.)
    parser.add_argument('--link-inputs', action='store_true', help='Hardlink immutable prepared photos/masks on the same filesystem')
    parser.add_argument('--minimum-free-gib', type=float, default=8.8)
    args = parser.parse_args()
    if not np.isfinite(args.minimum_free_gib) or args.minimum_free_gib < 0:
        parser.error('invalid free-space reserve')
    data = json.loads((args.inputs/'cameras.json').read_text())
    selected, report = selection(data['views'], np.load(args.inputs/'sparse_points.npy', allow_pickle=False),
        json.loads(args.support_report.read_text()), args.keep,
        json.loads(args.photo_map.read_text()) if args.photo_map else None, args.maximum_direction_gap)
    rows = [data['views'][i] for i in selected]
    paths = {'sparse_points.npy'}
    for row in rows:
        paths.update(row[key] for key in ['image', 'mask', 'texture_image'] if key in row)
    for name in paths:
        if Path(name).is_absolute() or not (args.inputs/name).resolve().is_relative_to(args.inputs.resolve()):
            raise ValueError('prepared input files must stay within the input directory')
    args.output.mkdir(parents=True, exist_ok=False)
    required = sum((args.inputs/name).stat().st_size for name in paths) if not args.link_inputs else 0
    if shutil.disk_usage(args.output).free < args.minimum_free_gib*2**30 + required:
        raise ValueError('output would cross the free-space reserve')
    for name in sorted(paths):
        target = args.output/name; target.parent.mkdir(parents=True, exist_ok=True)
        if args.link_inputs: os.link(args.inputs/name, target)
        else: shutil.copy2(args.inputs/name, target)
    report.update({'cameras_sha256': sha(args.inputs/'cameras.json'), 'support_report_sha256': sha(args.support_report),
        'photo_map_sha256': sha(args.photo_map) if args.photo_map else None, 'maximum_direction_gap': args.maximum_direction_gap,
        'limits': 'Caller must tie the support report to the same own recovery/photos. Name matching and hashes alone do not establish provenance or accuracy. Full-batch cameras/sparse points retained. Check excluded views too.'})
    (args.output/'selection.json').write_text(json.dumps(report, indent=2)+'\n')
    (args.output/'cameras.json').write_text(json.dumps(dict(data, views=rows), indent=2)+'\n')


if __name__ == '__main__':
    main()
