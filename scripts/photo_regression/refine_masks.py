#!/usr/bin/env python
"""Optional SAM2.1 boundary refinement with explicit coarse coverage checks.

Requires separately obtained pinned Apache source/checkpoint. No downloads,
angle priors, reference masks or new app dependency. Keeps the original coarse
mask on any rejected candidate; a completed run is not shape-quality acceptance.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sys
import threading
import time

for name in ['OMP_NUM_THREADS', 'OPENBLAS_NUM_THREADS', 'VECLIB_MAXIMUM_THREADS']:
    os.environ.setdefault(name, '2')
import cv2
import numpy as np
from PIL import Image, ImageOps

INVENTORY = Path(__file__).resolve().parents[2] / 'tests/evidence/sam21-refinement-source.json'


def digest(path):
    value = hashlib.sha256()
    with Path(path).open('rb') as stream:
        while block := stream.read(4 << 20):
            value.update(block)
    return value.hexdigest()


def foreground_cues(rgb, coarse):
    """Interior grid weighted by image contrast, rather than arbitrary grid centres."""
    mask = np.asarray(coarse, dtype=bool)
    if rgb.shape != (*mask.shape, 3) or not mask.any():
        raise ValueError('nonempty matching RGB/coarse mask required')
    h, w = mask.shape
    if min(h, w) < 6:
        raise ValueError('foreground crop too small')
    lab = cv2.cvtColor(rgb, cv2.COLOR_RGB2LAB).astype('float32')
    outside = ~cv2.dilate(mask.astype('uint8'), np.ones((11, 11), 'uint8')).astype(bool)
    if outside.sum() < 10:
        raise ValueError('insufficient known background around the coarse mask')
    contrast = np.linalg.norm(lab - np.median(lab[outside], axis=0), axis=2)
    coords, labels = [], []
    for iy in range(3):
        for ix in range(3):
            y0, y1 = iy * h // 3, (iy + 1) * h // 3
            x0, x1 = ix * w // 3, (ix + 1) * w // 3
            distance = cv2.distanceTransform(mask[y0:y1, x0:x1].astype('uint8'), cv2.DIST_L2, 3)
            colour = contrast[y0:y1, x0:x1]
            score = np.minimum(distance, 40) * np.minimum(colour, 100)
            score[(distance < 5) | (colour < 20)] = 0
            y, x = np.unravel_index(np.argmax(score), score.shape)
            if score[y, x] > 0:
                coords.append([x0 + int(x), y0 + int(y)]); labels.append(1)
    if not coords:
        raise ValueError('no contrast-supported foreground cue')
    for x, y in [[0, 0], [w - 1, 0], [0, h - 1], [w - 1, h - 1]]:
        if not mask[y, x]:
            coords.append([x, y]); labels.append(0)
    return coords, labels


def clean_candidate(mask):
    """Remove <9px islands and fill <=9px enclosed pinholes, using 8-connectivity.

    Do not close channels or fill arbitrary holes; real openings may be present.
    All edits count towards the caller's cleanup budget.
    """
    mask = np.asarray(mask, dtype='uint8')
    count, labels, stats, _ = cv2.connectedComponentsWithStats(mask, connectivity=8)
    ids = [k for k in range(1, count) if stats[k, cv2.CC_STAT_AREA] >= 9]
    clean = np.isin(labels, ids)
    count, holes, stats, _ = cv2.connectedComponentsWithStats((~clean).astype('uint8'), connectivity=8)
    border = set(np.unique(np.r_[holes[0], holes[-1], holes[:, 0], holes[:, -1]]))
    small = [k for k in range(1, count) if k not in border and stats[k, cv2.CC_STAT_AREA] <= 9]
    return clean | np.isin(holes, small)


def choose_candidate(masks, scores, coarse, coords, labels, minimum_coverage=.85, maximum_spill=.15, maximum_cleanup=.02):
    values = np.asarray(masks)
    if not np.isfinite(values).all() or np.any((values != 0) & (values != 1)):
        raise ValueError('binary finite masks required')
    masks, coarse, scores = values.astype(bool), np.asarray(coarse, dtype=bool), np.asarray(scores)
    if masks.ndim != 3 or masks.shape[1:] != coarse.shape or len(masks) != len(scores) or not coarse.any() or not np.isfinite(scores).all():
        raise ValueError('invalid mask candidates/scores/coarse foreground')
    points, cue_labels = np.asarray(coords), np.asarray(labels)
    if (points.shape != (len(labels), 2) or not np.issubdtype(points.dtype, np.integer)
            or np.any(points < 0) or np.any(points[:, 0] >= coarse.shape[1]) or np.any(points[:, 1] >= coarse.shape[0])
            or np.any((cue_labels != 0) & (cue_labels != 1)) or not np.any(cue_labels == 1)):
        raise ValueError('valid photographic foreground/background cues required')
    cleaned, reports = [], []
    for i, (mask, score) in enumerate(zip(masks, scores)):
        clean = clean_candidate(mask)
        delta = float(np.count_nonzero(clean != mask) / max(np.count_nonzero(mask), 1))
        coverage = float(np.count_nonzero(clean & coarse) / coarse.sum())
        spill = float(np.count_nonzero(clean & ~coarse) / coarse.sum())
        agrees = all(bool(clean[y, x]) == bool(label) for (x, y), label in zip(coords, labels))
        eligible = agrees and coverage >= minimum_coverage and spill <= maximum_spill and delta <= maximum_cleanup
        reports.append({'index': i, 'score': float(score), 'coverage': coverage, 'spill': spill,
                        'cleanup_fraction': delta, 'all_cues': agrees, 'eligible': eligible})
        cleaned.append(clean)
    eligible = [row for row in reports if row['eligible']]
    selected = max(eligible, key=lambda row: row['score'])['index'] if eligible else None
    return (cleaned[selected] if selected is not None else coarse.copy()), selected, reports


def exclude_progress_dependency():
    """Import-only boundary: image inference needs no video/progress functionality.

    Do not load the installed MPL/combined-license tqdm implementation. This
    original shim implements no progress API; accidental use fails immediately.
    """
    import importlib.machinery
    import types
    if any(name == 'tqdm' or name.startswith('tqdm.') for name in sys.modules):
        raise ValueError('run in a fresh process without an already loaded tqdm implementation')
    def unsupported(*args, **kwargs):
        raise RuntimeError('video/progress functionality is excluded from the image-only mask runner')
    class UnavailableProgress:
        # Optional Apache dependencies declare subclasses at import time. Permit
        # declarations, while refusing every actual progress object/operation.
        def __init__(self, *args, **kwargs):
            unsupported()
        write = staticmethod(unsupported)
        @classmethod
        def set_lock(cls, lock):
            # Import metadata only; no iteration, rendering or progress I/O.
            cls._import_lock = lock
        @classmethod
        def get_lock(cls):
            return cls._import_lock
    for name in ['tqdm', 'tqdm.auto', 'tqdm.std']:
        module = types.ModuleType(name)
        module.__file__ = __file__
        module.__spec__ = importlib.machinery.ModuleSpec(name, loader=None, is_package=name == 'tqdm')
        module.__path__ = []
        module.tqdm = UnavailableProgress
        module.trange = unsupported
        module.__version__ = '0.0.0'
        sys.modules[name] = module


def run(args):
    import resource
    inventory = json.loads(INVENTORY.read_text())
    for name, record in inventory['files'].items():
        if digest(args.source / name) != record['sha256']:
            raise ValueError(f'upstream source hash differs: {name}')
    if digest(args.checkpoint) != inventory['checkpoint']['sha256']:
        raise ValueError('expected pinned Apache SAM2.1 tiny checkpoint')
    def watch():
        while True:
            rss = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss * (1 if sys.platform == 'darwin' else 1024)
            if rss > args.maximum_rss_gib * 2**30:
                print('mask refiner exceeded RSS budget', flush=True); os._exit(86)
            time.sleep(.1)
    threading.Thread(target=watch, daemon=True).start()
    files = sorted(p for p in args.photos.iterdir() if p.suffix.lower() in {'.jpg', '.jpeg', '.png', '.tif', '.tiff'})
    if not files or len(files) > 1536:
        raise ValueError('expected 1..1536 photos')
    if args.limit:
        files = files[:args.limit]
    args.output.mkdir(parents=True, exist_ok=False)
    if any(name == 'sam2' or name.startswith('sam2.') for name in sys.modules):
        raise ValueError('run in a fresh process without another SAM source already imported')
    sys.path.insert(0, str(args.source))
    exclude_progress_dependency()
    import torch
    from sam2.build_sam import build_sam2
    from sam2.sam2_image_predictor import SAM2ImagePredictor
    torch.set_num_threads(2); torch.set_num_interop_threads(2); cv2.setNumThreads(2)
    model = build_sam2('configs/sam2.1/sam2.1_hiera_t.yaml', str(args.checkpoint), device='cpu', apply_postprocessing=False)
    reports = []; started = time.monotonic()
    for file in files:
        choices = [args.masks / (file.name + '.png'), args.masks / file.with_suffix('.png').name]
        path = next((p for p in choices if p.is_file()), None)
        if path is None:
            raise ValueError(f'no own coarse mask: {file.name}')
        with Image.open(file) as image, Image.open(path) as alpha:
            rgb = np.array(ImageOps.exif_transpose(image).convert('RGB')); coarse = np.array(alpha.convert('L')) > 127
        if coarse.shape != rgb.shape[:2] or not coarse.any():
            raise ValueError(f'coarse mask dimensions/foreground invalid: {file.name}')
        y, x = np.where(coarse); padding = args.padding_pixels
        a, b = max(0, x.min() - padding), max(0, y.min() - padding)
        c, d = min(rgb.shape[1], x.max() + 1 + padding), min(rgb.shape[0], y.max() + 1 + padding)
        crop, prior = rgb[b:d, a:c], coarse[b:d, a:c]
        selected, candidates, reason = None, [], None
        try:
            coords, labels = foreground_cues(crop, prior)
            predictor = SAM2ImagePredictor(model)
            with torch.inference_mode():
                predictor.set_image(crop)
                masks, scores, logits = predictor.predict(point_coords=np.array(coords, dtype='float32'),
                    point_labels=np.array(labels, dtype='int32'), box=np.array([x.min()-a,y.min()-b,x.max()-a,y.max()-b], dtype='float32'), multimask_output=True)
            if not np.isfinite(logits).all():
                raise ValueError('nonfinite SAM logits; retaining coarse mask')
            refined, selected, candidates = choose_candidate(masks, scores, prior, coords, labels,
                args.minimum_coverage, args.maximum_spill, args.maximum_cleanup)
            del predictor
        except ValueError as error:
            refined, reason = prior, str(error)
        full = np.zeros_like(coarse); full[b:d, a:c] = refined
        target = args.output / (file.name + '.png'); pending = target.with_suffix('.pending')
        Image.fromarray(full.astype('uint8') * 255).save(pending, format='PNG'); pending.replace(target)
        reports.append({'photo': file.name, 'selected': selected, 'coarse_fallback': selected is None,
            'reason': reason, 'candidates': candidates, 'rgb_sha256': digest(file), 'coarse_mask_sha256': digest(path), 'mask_sha256': digest(target)})
        pending = args.output / 'progress.pending'
        pending.write_text(json.dumps({'completed': len(reports), 'input': len(files), 'photos': reports}, indent=2)); pending.replace(args.output / 'progress.json')
        print('refined masks', len(reports), '/', len(files), flush=True)
    if any(getattr(module, '__file__', None) != __file__ for name, module in sys.modules.items() if name == 'tqdm' or name.startswith('tqdm.')):
        raise RuntimeError('unexpected progress implementation loaded')
    report = {'status': 'complete', 'quality_accepted': False, 'device': 'CPU F32, no AMP', 'seconds': time.monotonic()-started,
        'views': len(files), 'refined': sum(not v['coarse_fallback'] for v in reports), 'photos': reports,
        'tqdm_implementation_loaded': False, 'progress_api': 'excluded, fail-fast import shim only',
        'checkpoint_sha256': inventory['checkpoint']['sha256'], 'source_inventory_sha256': digest(INVENTORY),
        'peak_rss_bytes': resource.getrusage(resource.RUSAGE_SELF).ru_maxrss * (1 if sys.platform == 'darwin' else 1024),
        'parameters': {'padding_pixels': args.padding_pixels, 'minimum_coverage': args.minimum_coverage, 'maximum_spill': args.maximum_spill, 'maximum_cleanup': args.maximum_cleanup},
        'limits': 'Caller must supply own coarse masks. Coverage is relative to an uncertain prior, not mask/shape accuracy. No manual/reference pixel labels; fallback is explicit. No app provider or physical-scale claim.'}
    (args.output / 'report.json').write_text(json.dumps(report, indent=2)+'\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['photos', 'masks', 'source', 'checkpoint', 'output']:
        parser.add_argument('--'+name, type=Path, required=True)
    parser.add_argument('--padding-pixels', type=int, default=30)
    parser.add_argument('--minimum-coverage', type=float, default=.85)
    parser.add_argument('--maximum-spill', type=float, default=.15)
    parser.add_argument('--maximum-cleanup', type=float, default=.02)
    parser.add_argument('--maximum-rss-gib', type=float, default=3.)
    parser.add_argument('--limit', type=int, help='Optional small diagnostic prefix')
    args = parser.parse_args()
    if (not 1 <= args.padding_pixels <= 512 or not 0 < args.minimum_coverage <= 1 or not 0 <= args.maximum_spill <= 1
            or not 0 <= args.maximum_cleanup <= .2 or not np.isfinite(args.maximum_rss_gib) or args.maximum_rss_gib <= 0
            or args.limit is not None and args.limit < 1):
        parser.error('invalid mask/refinement budget')
    run(args)


if __name__ == '__main__':
    main()
