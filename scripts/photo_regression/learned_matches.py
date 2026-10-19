#!/usr/bin/env python
"""Optional offline ALIKED/LightGlue foreground matching; no pose/angle prior.

Requires separately obtained, pinned permissively licensed code and weights.
No network downloads or app dependency. CPU F32 is intentional: MPS sampling
and cached-layout parity need independent validation for each implementation.
"""
import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import sys
import time

import numpy as np
from PIL import Image

# LightGlue eb42fee2d71449efb0aa5c10549752b5d75384d8 (Apache-2.0);
# ALIKED model 683d7c65197395c0b3f01ebe76e1084a27e73a65 (BSD-3-Clause).
# Full license/source records: tests/evidence/camera-recovery-source-review.json.
SOURCE_HASHES = {
    '__init__.py': 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855',
    'aliked.py': '60b24397c52e8a08d838828967cd03f5d6066c1f7c3c433805bf11dcb0d5228e',
    'lightglue.py': 'dcb75b9cad1985c5e7a537c512d4318d47fbcc99df75b39a513de60e027c039f',
    'utils.py': 'c6d9260656a24d826128b68c95d0b7798106d04f4244a77f3b1856573f58e690',
}
EXTRACTOR_HASH = 'ddf3abbf38e86f6a74540d214e1a9712c54b2d8551abc864542199f2347d7332'
MATCHER_HASH = 'd975e965b105311a6143194852297dff4f02aea5cc2e10cecfed966ca0e22503'


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def atomic_npz(path, **arrays):
    pending = path.with_suffix('.pending')
    with pending.open('wb') as stream:
        np.savez_compressed(stream, **arrays)
    pending.replace(path)


def square_crop(mask, padding=.15):
    ys, xs = np.where(np.asarray(mask) > 127)
    if not len(xs):
        raise ValueError('empty foreground mask')
    side = int(np.ceil(max(xs.max() - xs.min() + 1, ys.max() - ys.min() + 1) * (1 + 2 * padding)))
    left = int(np.floor((xs.min() + xs.max() + 1 - side) / 2))
    top = int(np.floor((ys.min() + ys.max() + 1 - side) / 2))
    # Padding may extend outside the photo; PIL crop plus white compositing is
    # deliberate. No foreground is clipped to force a square inside the frame.
    return [left, top, left + side, top + side]


def source_keypoints(points, crop, resolution):
    scale = resolution / (crop[2] - crop[0])
    return (np.asarray(points) + .5) / scale + np.asarray(crop[:2]) - .5


def candidate_graph(descriptors, neighbors=6, temporal_radius=2, overlap=None):
    descriptors = np.asarray(descriptors, dtype=float)
    if descriptors.ndim != 2 or not np.isfinite(descriptors).all():
        raise ValueError('finite descriptor matrix required')
    if neighbors < 0 or temporal_radius < 0:
        raise ValueError('negative candidate budget')
    n = len(descriptors)
    unit = descriptors / np.maximum(np.linalg.norm(descriptors, axis=1, keepdims=True), 1e-12)
    similarity = unit @ unit.T
    pairs = set()
    for i in range(n):
        order = np.argsort(-similarity[i], kind='stable')
        choices = [int(j) for j in order if j != i][:neighbors]
        choices += list(range(max(0, i - temporal_radius), min(n, i + temporal_radius + 1)))
        if overlap is not None:
            choices += [j for score, j in sorted(overlap[i], reverse=True)[:4] if score >= 12]
        for j in choices:
            if i != j:
                pairs.add(tuple(sorted((i, j))))
    return sorted(pairs)


def validated_pair(matches, first_count, second_count):
    matches = np.asarray(matches)
    if matches.size == 0:
        return np.empty((0, 2), dtype=np.int64)
    if (matches.ndim != 2 or matches.shape[1] != 2 or
            not np.issubdtype(matches.dtype, np.integer) or
            np.any(matches < 0) or np.any(matches[:, 0] >= first_count) or
            np.any(matches[:, 1] >= second_count)):
        raise ValueError('invalid cached pair indices')
    if (len(np.unique(matches[:, 0])) != len(matches) or
            len(np.unique(matches[:, 1])) != len(matches)):
        raise ValueError('cached matches must be one-to-one')
    return matches.astype(np.int64)


@contextmanager
def local_weights(torch, extractor, matcher):
    previous = torch.hub.load_state_dict_from_url
    def load(url, **kwargs):
        if 'aliked_lightglue' in url:
            path = matcher
        elif 'aliked-n16rot' in url:
            path = extractor
        else:
            raise ValueError('unexpected model URL; network access refused')
        return torch.load(path, map_location='cpu', weights_only=True)
    torch.hub.load_state_dict_from_url = load
    try:
        yield
    finally:
        torch.hub.load_state_dict_from_url = previous


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['source', 'extractor', 'matcher', 'template', 'photos', 'masks', 'output']:
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--resolution', type=int, default=768)
    parser.add_argument('--maximum-keypoints', type=int, default=512)
    parser.add_argument('--neighbors', type=int, default=6)
    parser.add_argument('--temporal-radius', type=int, default=2, help='index neighbours only; no equal-angle assumption')
    parser.add_argument('--reuse-overlap', action='store_true', help='Also use up to four neighbours from template matches; disclose their cost separately')
    args = parser.parse_args()
    if not 64 <= args.resolution <= 2048 or not 32 <= args.maximum_keypoints <= 4096 or args.neighbors < 0 or args.temporal_radius < 0:
        parser.error('invalid resolution, keypoint or pair budget')
    for name, expected in SOURCE_HASHES.items():
        if sha(args.source / 'lightglue' / name) != expected:
            parser.error('unexpected external source hash: ' + name)
    for path, expected in [(args.extractor, EXTRACTOR_HASH), (args.matcher, MATCHER_HASH)]:
        if sha(path) != expected:
            parser.error('unexpected checkpoint hash: ' + str(path))
    template = json.loads(args.template.read_text())
    names, lens = template['names'], template['lens']
    if len(names) != len(set(names)) or any(Path(name).name != name for name in names):
        parser.error('unique basename-only photo names required')
    if not 2 <= len(names) <= 1536:
        parser.error('expected 2..1536 photos')
    expected_size = (lens['width'], lens['height'])
    ledger = []
    for name in names:
        photo, mask = args.photos / name, args.masks / (name + '.png')
        with Image.open(photo) as rgb, Image.open(mask) as mk:
            if rgb.size != expected_size or mk.size != expected_size or rgb.getexif().get(274, 1) != 1:
                parser.error('normalize orientation and lens/photo/mask dimensions first: ' + name)
            box = square_crop(np.array(mk.convert('L')))
        ledger.append({'name': name, 'crop': box, 'photo_sha256': sha(photo), 'mask_sha256': sha(mask)})
    config = {'photos': ledger, 'resolution': args.resolution, 'extractor_hash': EXTRACTOR_HASH,
              'maximum_extracted_keypoints': 1200, 'detection_threshold': .05, 'source_hashes': SOURCE_HASHES}
    fingerprint = hashlib.sha256(json.dumps(config, sort_keys=True).encode()).hexdigest()
    args.output.mkdir(parents=True, exist_ok=True)
    cache = args.output / ('features-' + fingerprint[:16]);cache.mkdir(exist_ok=True)
    (cache / 'ledger.json').write_text(json.dumps(config, indent=2) + '\n')
    os.environ.setdefault('OMP_NUM_THREADS', '2')
    import torch
    torch.set_num_threads(2)
    torch.set_num_interop_threads(2)
    sys.path.insert(0, str(args.source.resolve()))
    from lightglue.aliked import ALIKED
    from lightglue.lightglue import LightGlue
    with local_weights(torch, args.extractor, args.matcher):
        extractor = ALIKED(model_name='aliked-n16rot', max_num_keypoints=1200, detection_threshold=.05).eval()
        matcher = LightGlue(features='aliked', depth_confidence=-1, width_confidence=-1).eval()
    start = time.monotonic();features, keys, pooled = [], [], [];cache_hits = 0
    for i, entry in enumerate(ledger):
        cached = cache / f'{i:04}.npz'
        if cached.exists():
            with np.load(cached, allow_pickle=False) as arr:
                f = {k: torch.from_numpy(arr[k]) for k in arr.files}
            cache_hits += 1
        else:
            with Image.open(args.photos / entry['name']) as rgb, Image.open(args.masks / (entry['name'] + '.png')) as mk:
                im = rgb.convert('RGB').crop(entry['crop']);mk = mk.convert('L').crop(entry['crop'])
                bg = Image.new('RGB', im.size, 'white');bg.paste(im, (0, 0), mk)
                ten = torch.from_numpy(np.array(bg.resize((args.resolution, args.resolution), Image.Resampling.LANCZOS)).copy()).permute(2, 0, 1).float() / 255
                valid_mask = np.array(mk.resize((args.resolution, args.resolution), Image.Resampling.NEAREST)) > 127
            with torch.inference_mode():
                f = extractor.extract(ten, resize=None)
            positions = np.rint(f['keypoints'][0].numpy()).astype(int)
            valid = ((positions >= 0) & (positions < args.resolution)).all(axis=1)
            ids = np.flatnonzero(valid);valid[ids] &= valid_mask[positions[ids, 1], positions[ids, 0]]
            for key in ['keypoints', 'descriptors', 'keypoint_scores']:
                f[key] = f[key][:, valid]
            atomic_npz(cached, **{k: v.numpy() for k, v in f.items()})
        if f['keypoints'].shape[1] > args.maximum_keypoints:
            selected = torch.argsort(f['keypoint_scores'][0], descending=True)[:args.maximum_keypoints]
            for key in ['keypoints', 'descriptors', 'keypoint_scores']:
                f[key] = f[key][:, selected]
        keys.append(source_keypoints(f['keypoints'][0].numpy(), entry['crop'], args.resolution).tolist())
        descriptors = f['descriptors'][0].numpy()
        pooled.append(descriptors.mean(0) if len(descriptors) else np.zeros(f['descriptors'].shape[-1]))
        features.append(f)
        print('features', i + 1, '/', len(names), len(keys[-1]), flush=True)
    extraction_seconds = time.monotonic() - start
    overlap = None
    if args.reuse_overlap:
        overlap = [[] for _ in names]
        for pair in template['pairs']:
            a, b = pair['first'], pair['second'];score = len(pair['matches'])
            overlap[a].append((score, b));overlap[b].append((score, a))
    candidates = candidate_graph(pooled, args.neighbors, args.temporal_radius, overlap)
    pair_config = {'features': fingerprint, 'maximum_keypoints': args.maximum_keypoints,
                   'matcher_hash': MATCHER_HASH, 'source_hashes': SOURCE_HASHES,
                   'device': 'CPU F32', 'depth_confidence': -1, 'width_confidence': -1}
    pair_fingerprint = hashlib.sha256(json.dumps(pair_config, sort_keys=True).encode()).hexdigest()
    pair_cache = args.output / ('pairs-' + pair_fingerprint[:16]);pair_cache.mkdir(exist_ok=True)
    (pair_cache / 'ledger.json').write_text(json.dumps(pair_config, indent=2) + '\n')
    pairs = [];match_start = time.monotonic();pair_cache_hits = 0
    for index, (i, j) in enumerate(candidates):
        if min(len(keys[i]), len(keys[j])) < 8:
            continue
        cached_pair = pair_cache / f'{i:04}-{j:04}.npz'
        if cached_pair.exists():
            with np.load(cached_pair, allow_pickle=False) as arr:
                matches = validated_pair(arr['matches'], len(keys[i]), len(keys[j]))
            pair_cache_hits += 1
        else:
            with torch.inference_mode():
                matches = matcher({'image0': features[i], 'image1': features[j]})['matches'][0].numpy()
            matches = validated_pair(matches, len(keys[i]), len(keys[j]))
            atomic_npz(cached_pair, matches=matches)
        if len(matches) >= 8:
            pairs.append({'first': i, 'second': j, 'matches': matches.astype(int).tolist()})
        if index % 25 == 0:
            print('pairs', index + 1, '/', len(candidates), flush=True)
    (args.output / 'matches.json').write_text(json.dumps({'schema': 'crispmesh_turntable_matches_v1',
        'names': names, 'lens': lens, 'keypoints': keys, 'pairs': pairs,
        'note': 'Own foreground ALIKED/LightGlue correspondences; original integer pixel-centre coordinates; no supplied poses/depth/geometry or equal-angle assumption.'}) + '\n')
    (args.output / 'report.json').write_text(json.dumps({'photos': len(names), 'candidate_pairs': len(candidates), 'matched_pairs': len(pairs),
        'cache_hits': cache_hits, 'pair_cache_hits': pair_cache_hits, 'extraction_seconds': extraction_seconds, 'matching_seconds': time.monotonic() - match_start,
        'device': 'CPU F32', 'resolution': args.resolution, 'maximum_match_keypoints': args.maximum_keypoints,
        'neighbors': args.neighbors, 'temporal_radius': args.temporal_radius, 'reuse_overlap': args.reuse_overlap,
        'pairing_limits': 'Pooled similarity is quadratic, costly learned pair matching is bounded by N times candidate budget. Reused overlap preprocessing cost excluded.',
        'weight_hashes': {'extractor': EXTRACTOR_HASH, 'matcher': MATCHER_HASH},
        'code_hashes': SOURCE_HASHES, 'reference_inputs_used': False}, indent=2) + '\n')


if __name__ == '__main__':
    main()
