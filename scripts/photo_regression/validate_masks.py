#!/usr/bin/env python
"""Gate own refined masks before native import; preserve uncertain coarse masks.

Checks the existing native dark-hole cleanup budget and image support for new
interior boundaries. Does not prove segmentation or 3D shape accuracy.
"""
import argparse
import hashlib
import json
from pathlib import Path

import cv2
import numpy as np
from PIL import Image, ImageOps


def dark_hole_fraction(rgb, mask, luminance_margin=2):
    """Native cleanup: enclosed 4-connected BG <=2048px, all pixels <=128.

    The extra luminance margin conservatively covers small JPEG-decoder colour
    differences. This predicts a rejection; it never relaxes native cleanup.
    """
    mask = np.asarray(mask, dtype=bool)
    gray = np.array(Image.fromarray(rgb).convert('L'))
    count, labels, stats, _ = cv2.connectedComponentsWithStats((~mask).astype('uint8'), connectivity=4)
    border = set(np.unique(np.r_[labels[0], labels[-1], labels[:, 0], labels[:, -1]]))
    maximum = np.zeros(count, dtype='uint8'); np.maximum.at(maximum, labels.ravel(), gray.ravel())
    filled = sum(int(stats[k, cv2.CC_STAT_AREA]) for k in range(1, count)
                 if k not in border and stats[k, cv2.CC_STAT_AREA] <= 2048 and maximum[k] <= 128 + luminance_margin)
    return filled / max(int(mask.sum()), 1)


def interior_boundary_support(rgb, coarse, refined):
    distance = cv2.distanceTransform(coarse.astype('uint8'), cv2.DIST_L2, 3)
    boundary = cv2.morphologyEx(refined.astype('uint8'), cv2.MORPH_GRADIENT, np.ones((3, 3), 'uint8')) > 0
    interior = boundary & (distance > 3)
    if interior.sum() < 10:
        return 1., int(interior.sum())
    lab = cv2.GaussianBlur(cv2.cvtColor(rgb, cv2.COLOR_RGB2LAB).astype('float32'), (0, 0), .8)
    dx = cv2.Sobel(lab, cv2.CV_32F, 1, 0, ksize=3, scale=1/8)
    dy = cv2.Sobel(lab, cv2.CV_32F, 0, 1, ksize=3, scale=1/8)
    gradient = np.sqrt(np.sum(dx*dx + dy*dy, axis=2))
    nearby = cv2.dilate(gradient, np.ones((5, 5), 'uint8'))
    return float(np.mean(nearby[interior] >= 6.)), int(interior.sum())


def validate_refinement(rgb, coarse, refined, cleanup_budget=.02, minimum_boundary_support=.5):
    coarse, refined = np.asarray(coarse, dtype=bool), np.asarray(refined, dtype=bool)
    if (rgb.ndim != 3 or rgb.shape[2] != 3 or coarse.shape != rgb.shape[:2] or refined.shape != coarse.shape
            or not coarse.any()):
        raise ValueError('matching nonempty own RGB/masks required')
    y, x = np.where(coarse | refined)
    a, b = max(0, x.min()-6), max(0, y.min()-6)
    c, d = min(rgb.shape[1], x.max()+7), min(rgb.shape[0], y.max()+7)
    image, prior, candidate = rgb[b:d, a:c], coarse[b:d, a:c], refined[b:d, a:c]
    fraction = dark_hole_fraction(image, candidate)
    support, pixels = interior_boundary_support(image, prior, candidate)
    reasons = ['empty_refined_mask'] if not refined.any() else []
    if fraction > cleanup_budget: reasons.append('native_dark_hole_budget')
    if support < minimum_boundary_support: reasons.append('new_interior_boundaries_without_image_support')
    if reasons and dark_hole_fraction(image, prior) > cleanup_budget:
        raise ValueError('coarse fallback also exceeds native cleanup budget')
    return (coarse.copy() if reasons else refined.copy()), {
        'coarse_fallback': bool(reasons), 'reasons': reasons,
        'predicted_native_dark_hole_fraction_with_luminance_margin2': fraction,
        'new_interior_boundary_pixels': pixels, 'new_interior_boundary_image_support': support,
        'quality_accepted': False}


def digest(path):
    h = hashlib.sha256()
    with Path(path).open('rb') as stream:
        while block := stream.read(4 << 20): h.update(block)
    return h.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['photos', 'coarse-masks', 'refined-masks', 'output']:
        parser.add_argument('--'+name, type=Path, required=True)
    parser.add_argument('--cleanup-budget', type=float, default=.02)
    parser.add_argument('--minimum-boundary-support', type=float, default=.5)
    args = parser.parse_args()
    if not 0 <= args.cleanup_budget <= .05 or not 0 <= args.minimum_boundary_support <= 1:
        parser.error('invalid mask validation budget')
    cv2.setNumThreads(2)
    args.output.mkdir(parents=True, exist_ok=False); rows = []
    for file in sorted(args.photos.iterdir()):
        if file.suffix.lower() not in {'.jpg', '.jpeg', '.png', '.tif', '.tiff'}: continue
        coarse_path, refined_path = args.coarse_masks/(file.name+'.png'), args.refined_masks/(file.name+'.png')
        with Image.open(file) as image, Image.open(coarse_path) as alpha, Image.open(refined_path) as refined:
            rgb = np.array(ImageOps.exif_transpose(image).convert('RGB'))
            coarse, candidate = np.array(alpha.convert('L')) > 127, np.array(refined.convert('L')) > 127
        result, report = validate_refinement(rgb, coarse, candidate, args.cleanup_budget, args.minimum_boundary_support)
        target = args.output/(file.name+'.png'); pending = target.with_suffix('.pending')
        Image.fromarray(result.astype('uint8')*255).save(pending, format='PNG'); pending.replace(target)
        report.update({'photo': file.name, 'rgb_sha256': digest(file), 'coarse_sha256': digest(coarse_path),
                       'refined_sha256': digest(refined_path), 'output_sha256': digest(target)})
        rows.append(report)
    if not rows: raise ValueError('no photos validated')
    (args.output/'report.json').write_text(json.dumps({'views': len(rows), 'coarse_fallbacks': sum(v['coarse_fallback'] for v in rows),
        'cleanup_budget': args.cleanup_budget, 'minimum_boundary_support': args.minimum_boundary_support,
        'quality_accepted': False, 'reference_inputs_used': False, 'photos': rows,
        'limits': 'Conservative photometric/cleanup checks, not shape accuracy. Real low-contrast holes may revert to the coarse mask; inspect and keep alternative paths.'}, indent=2)+'\n')


if __name__ == '__main__': main()
