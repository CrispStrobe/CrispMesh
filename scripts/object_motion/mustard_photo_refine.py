#!/usr/bin/env python3
"""Training-only, conservative photo refinement of the reviewed mustard SAM support.

The fixed GrabCut trim can change only a three-pixel band inside the prior mask.
All labels and other pixels more than three pixels from its edge are retained.
This is a coarse support candidate, not a certified silhouette.
"""

import argparse
import hashlib
import json
from pathlib import Path
import shutil

import cv2
import numpy as np
from PIL import Image, ImageDraw


TRAIN_ROOT = Path('/Volumes/backups/code/crispmesh-data/mustard-sfm-train-001')
EXPECTED_STAGE_SHA = 'bbcba624f6a51f82e0ebdebf6e7e4cda10804bc8df34cfd0d5b3242983a225d0'
MIN_FREE = 10 * 1024**3
MAX_OUTPUT = 128 * 1024**2
ROI = (470, 290, 770, 660)
TRIM_PX = 3
BACKGROUND_PX = 8
ITERATIONS = 3
CAP_PRESERVE_Y_EXCLUSIVE = 435


def sha(path):
    h = hashlib.sha256()
    with open(path, 'rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def refine(rgb, prior):
    if rgb.shape != (1024, 1280, 3) or rgb.dtype != np.uint8:
        raise ValueError('wrong photo shape or type')
    if prior.shape != rgb.shape[:2] or prior.dtype != np.uint8 or not np.isin(prior, [0, 255]).all():
        raise ValueError('prior must be a full-size binary mask')
    x0, y0, x1, y1 = ROI
    if np.any(prior[:y0]) or np.any(prior[y1:]) or np.any(prior[:, :x0]) or np.any(prior[:, x1:]):
        raise ValueError('prior escapes fixed ROI')
    old = prior[y0:y1, x0:x1] > 0
    if not 10_000 <= int(old.sum()) <= 30_000:
        raise ValueError('unexpected prior area')
    trim_kernel = cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (2 * TRIM_PX + 1,) * 2)
    bg_kernel = cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (2 * BACKGROUND_PX + 1,) * 2)
    core = cv2.erode(old.astype(np.uint8), trim_kernel) > 0
    expanded = cv2.dilate(old.astype(np.uint8), bg_kernel) > 0
    labels = np.full(old.shape, cv2.GC_PR_BGD, dtype=np.uint8)
    labels[~expanded] = cv2.GC_BGD
    labels[old] = cv2.GC_PR_FGD
    labels[core] = cv2.GC_FGD
    bg_model = np.zeros((1, 65), np.float64)
    fg_model = np.zeros((1, 65), np.float64)
    crop = cv2.cvtColor(rgb[y0:y1, x0:x1], cv2.COLOR_RGB2BGR)
    cv2.grabCut(crop, labels, None, bg_model, fg_model, ITERATIONS, cv2.GC_INIT_WITH_MASK)
    retained = old & (core | (labels == cv2.GC_FGD) | (labels == cv2.GC_PR_FGD))
    retained[:CAP_PRESERVE_Y_EXCLUSIVE - y0] = old[:CAP_PRESERVE_Y_EXCLUSIVE - y0]
    removed = int(old.sum() - retained.sum())
    if removed > int(old.sum() * .08):
        raise ValueError(f'trim exceeds 8 percent: {removed}')
    result = np.zeros(prior.shape, np.uint8)
    result[y0:y1, x0:x1] = retained.astype(np.uint8) * 255
    return result, {'prior_pixels': int(old.sum()), 'retained_pixels': int(retained.sum()),
                    'trimmed_pixels': removed, 'core_pixels': int(core.sum())}


def run(output):
    output = Path(output)
    if output.exists() or output.is_symlink():
        raise FileExistsError(output)
    if not str(output.resolve()).startswith('/Volumes/backups/code/crispmesh-data/'):
        raise ValueError('output must be on external data volume')
    if min(shutil.disk_usage('.').free,
           shutil.disk_usage('/Volumes/backups').free) < MIN_FREE:
        raise RuntimeError('10 GiB free-space floor')
    report_path = TRAIN_ROOT / 'stage-report.json'
    if sha(report_path) != EXPECTED_STAGE_SHA:
        raise ValueError('stage report digest changed')
    stage = json.loads(report_path.read_text())
    rows = stage['cleaned_masks']
    if len(rows) != 48 or set(p.name for p in (TRAIN_ROOT / 'images').iterdir()) != set(rows):
        raise ValueError('training inventory mismatch')
    if set(stage['train_photo_sha256']) != set(rows):
        raise ValueError('photo hash inventory mismatch')
    if set(p.name for p in (TRAIN_ROOT / 'masks').iterdir()) != {n + '.png' for n in rows}:
        raise ValueError('mask inventory mismatch')
    output.mkdir(parents=True)
    masks = output / 'masks'
    masks.mkdir()
    report = {'schema': 'mustard_photo_refine_v1', 'status': 'running',
              'role': 'coarse foreground support; unreviewed; no geometry score',
              'stage_report_sha256': EXPECTED_STAGE_SHA,
              'accepted_sam_inventory_sha256': stage['inventory_sha256'],
              'accepted_sam_visual_qa_sha256': stage['qa_sha256'],
              'runner_sha256': sha(__file__),
              'policy': {'roi_xyxy': ROI, 'trim_px': TRIM_PX,
                         'background_px': BACKGROUND_PX, 'grabcut_iterations': ITERATIONS,
                         'cap_preserve_y_exclusive': CAP_PRESERVE_Y_EXCLUSIVE,
                         'max_removed_fraction': .08, 'no_added_pixels': True},
              'images': []}
    sheet = Image.new('RGB', (8 * 200, 6 * 230), 'white')
    draw = ImageDraw.Draw(sheet)
    try:
        for i, name in enumerate(sorted(rows)):
            photo_path = TRAIN_ROOT / 'images' / name
            mask_path = TRAIN_ROOT / 'masks' / (name + '.png')
            if sha(mask_path) != rows[name]['sha256']:
                raise ValueError(f'prior mask changed: {name}')
            photo_sha = sha(photo_path)
            if photo_sha != stage['train_photo_sha256'][name]:
                raise ValueError(f'training photo changed: {name}')
            rgb = np.asarray(Image.open(photo_path).convert('RGB'))
            prior = np.asarray(Image.open(mask_path).convert('L'))
            candidate, stats = refine(rgb, prior)
            destination = masks / (name + '.png')
            Image.fromarray(candidate).save(destination)
            report['images'].append({'name': name, 'photo_sha256': photo_sha,
                                     'prior_sha256': rows[name]['sha256'],
                                     'candidate_sha256': sha(destination), **stats})
            x0, y0, x1, y1 = ROI
            crop = rgb[y0:y1, x0:x1].copy()
            rim = (prior[y0:y1, x0:x1] > 0) & (candidate[y0:y1, x0:x1] == 0)
            crop[rim] = [255, 0, 255]
            tile = Image.fromarray(crop)
            tile.thumbnail((196, 205), Image.Resampling.LANCZOS)
            x, y = (i % 8) * 200, (i // 8) * 230
            sheet.paste(tile, (x + (200 - tile.width) // 2, y))
            draw.text((x + 4, y + 207), name, fill='black')
            if sum(p.stat().st_size for p in output.rglob('*') if p.is_file()) > MAX_OUTPUT:
                raise RuntimeError('output cap')
        sheet.save(output / 'trim_review.jpg', quality=92)
        report['sheet_sha256'] = sha(output / 'trim_review.jpg')
        report['status'] = 'generated_unreviewed'
    except BaseException as error:
        report['status'] = 'failed_unusable'
        report['error'] = str(error)
        raise
    finally:
        (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', required=True)
    print(json.dumps({'status': run(parser.parse_args().output)['status']}))
