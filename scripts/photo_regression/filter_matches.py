#!/usr/bin/env python
"""Remove cached own feature matches outside foreground or near its boundary.

Only correspondence edges are removed: keypoint arrays, their indices, lens and
source coordinates remain unchanged. Does not add features missed by the initial
extractor. No camera poses, angle prior or reference data is consumed.
"""
import argparse
import hashlib
import json
from pathlib import Path

import cv2
import numpy as np
from PIL import Image
if __package__:
    from .learned_matches import validated_pair
else:
    from learned_matches import validated_pair


def foreground_flags(points, mask, margin=0.):
    points, mask = np.asarray(points, dtype=float), np.asarray(mask, dtype=bool)
    if points.ndim != 2 or points.shape[1] != 2 or not np.isfinite(points).all() or mask.ndim != 2 or not mask.size:
        raise ValueError('finite Nx2 source coordinates and 2D mask required')
    if not np.isfinite(margin) or margin < 0:
        raise ValueError('nonnegative finite pixel margin required')
    distance = cv2.distanceTransform(mask.astype('uint8'), cv2.DIST_L2, 3)
    # Bound before integer conversion, including extremely distant finite points.
    valid = ((points[:, 0] >= -.5) & (points[:, 0] < mask.shape[1]-.5)
             & (points[:, 1] >= -.5) & (points[:, 1] < mask.shape[0]-.5))
    ids = np.flatnonzero(valid); xy = np.floor(points[ids]+.5).astype(int)
    result = np.zeros(len(points), bool)
    result[ids] = distance[xy[:, 1], xy[:, 0]] > margin
    return result


def filter_pair(matches, first, second):
    pair = validated_pair(matches, len(first), len(second))
    return pair[first[pair[:, 0]] & second[pair[:, 1]]].tolist()


def sha(path):
    result = hashlib.sha256()
    with Path(path).open('rb') as stream:
        while data := stream.read(4 << 20): result.update(data)
    return result.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['matches', 'masks', 'output']:
        parser.add_argument('--'+name, type=Path, required=True)
    parser.add_argument('--margin-pixels', type=float, default=0.)
    parser.add_argument('--minimum-pair-matches', type=int, default=15)
    args = parser.parse_args()
    if not np.isfinite(args.margin_pixels) or args.margin_pixels < 0 or args.minimum_pair_matches < 5:
        parser.error('invalid foreground/match budget')
    if args.output.exists(): parser.error('output must be fresh')
    cv2.setNumThreads(2)
    data = json.loads(args.matches.read_text()); names = data['names']; points = data['keypoints']
    if len(set(names)) != len(names) or len(points) != len(names) or not 3 <= len(names) <= 1536:
        raise ValueError('expected uniquely named own photos/keypoint arrays')
    flags, hashes = [], {}
    for name, values in zip(names, points):
        if Path(name).name != name: raise ValueError('source names must be basenames')
        choices = [args.masks/(name+'.png'), args.masks/Path(name).with_suffix('.png').name]
        path = next((p for p in choices if p.is_file()), None)
        if path is None: raise ValueError(f'no own mask for {name}')
        with Image.open(path) as image: mask = np.array(image.convert('L')) > 127
        if mask.shape != (data['lens']['height'], data['lens']['width']):
            raise ValueError('mask must use the same original frame as the feature cache/lens')
        flags.append(foreground_flags(values, mask, args.margin_pixels)); hashes[name] = sha(path)
    before = sum(len(row['matches']) for row in data['pairs']); pairs = []; seen = set()
    for row in data['pairs']:
        a, b = row['first'], row['second']
        if not isinstance(a, int) or not isinstance(b, int) or not 0 <= a < len(names) or not 0 <= b < len(names) or a == b or tuple(sorted((a, b))) in seen:
            raise ValueError('invalid/duplicate photo pair')
        seen.add(tuple(sorted((a, b))))
        kept = filter_pair(row['matches'], flags[a], flags[b])
        if len(kept) >= args.minimum_pair_matches: pairs.append(dict(row, matches=kept))
    if not pairs: raise ValueError('no camera pairs survive foreground filtering')
    data['mask_filter'] = {'margin_pixels': args.margin_pixels, 'minimum_pair_matches': args.minimum_pair_matches,
        'input_matches': before, 'retained_matches': sum(len(p['matches']) for p in pairs),
        'input_pairs': len(data['pairs']), 'retained_pairs': len(pairs), 'source_cache_sha256': sha(args.matches),
        'mask_sha256': hashes, 'keypoints_indices_lens_coordinates_unchanged': True,
        'reference_inputs_used': False, 'quality_accepted': False,
        'limits': 'Foreground filtering only; reused extractor/candidate graph, not a new cold-start matching run or camera accuracy proof.'}
    data['pairs'] = pairs
    pending = args.output.with_suffix('.pending'); pending.write_text(json.dumps(data)); pending.replace(args.output)
    print(json.dumps({k:v for k,v in data['mask_filter'].items() if k != 'mask_sha256'}, indent=2))


if __name__ == '__main__': main()
