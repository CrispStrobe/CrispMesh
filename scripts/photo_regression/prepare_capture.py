#!/usr/bin/env python
"""Automatic own foreground instance choice and shared crop; no manual pixel boxes."""
import argparse
import json
import math
import os
import shutil
from pathlib import Path
import cv2
import numpy as np
from PIL import Image, ImageOps


def choose_instance(masks, maximum_fraction=.5):
    choices = []
    for name, mask in masks:
        h, w = mask.shape; y, x = np.where(mask > 127)
        if len(x) < 100 or len(x)/(w*h) > maximum_fraction: continue
        score = (len(x)/(w*h))**.5 / (.03 + (float(x.mean())/w-.5)**2 + (float(y.mean())/h-.5)**2)
        choices.append((score, name, mask, [int(x.min()), int(y.min()), int(x.max()+1), int(y.max()+1)]))
    return max(choices, key=lambda row: (row[0], row[1])) if choices else None


def common_crop(boxes, width, height):
    boxes = np.asarray(boxes, dtype=int)
    a, b = boxes[:, :2].min(0); c, d = boxes[:, 2:].max(0)
    pad = int(max(c-a, d-b)*.08)
    return [max(0, int(a-pad)), max(0, int(b-pad)), min(width, int(c+pad)), min(height, int(d+pad))]


def initial_lens(width, height, crop, calibration=None, fov=None):
    a, b, c, d = crop
    if calibration is None:
        # Explicit initial hypothesis, subsequently refined from own correspondences.
        if fov is not None and not 1 < fov < 179: raise ValueError('invalid horizontal field of view')
        f = width/(2*math.tan(math.radians(fov)/2)) if fov is not None else .9*max(width, height); cx, cy = (width-1)/2, (height-1)/2; k = 0.
    else:
        if (calibration['calibration_width'], calibration['calibration_height']) != (width, height):
            raise ValueError('calibration must describe the upright original frame')
        f = calibration['fx']; cx, cy = calibration['cx'], calibration['cy']; k = calibration.get('k1', 0.)
        if calibration.get('fy', f) != f or calibration.get('k2', 0.) or calibration.get('k3', 0.):
            raise ValueError('capture radial path needs equal focal lengths and k2=k3=0')
    if not np.isfinite([f, cx, cy, k]).all() or f <= 0: raise ValueError('invalid initial lens')
    return {'width': c-a, 'height': d-b, 'pixels': [f, f, cx-a, cy-b], 'k': [k, 0., 0.]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['photos', 'output']: parser.add_argument('--'+name, type=Path, required=True)
    source=parser.add_mutually_exclusive_group(required=True)
    source.add_argument('--instances',type=Path)
    source.add_argument('--masks',type=Path,help='Own full-frame masks, including the app crop preview masks')
    parser.add_argument('--calibration', type=Path)
    parser.add_argument('--fov', type=float)
    parser.add_argument('--maximum-foreground-fraction',type=float,default=.5)
    parser.add_argument('--shared-crop',action='store_true',help='Optional global crop before feature recovery; otherwise preserve the original frame')
    args = parser.parse_args(); cv2.setNumThreads(2)
    if not .1<=args.maximum_foreground_fraction<=.9:parser.error('invalid foreground area limit')
    args.output.mkdir(parents=True, exist_ok=False)
    for name in ['photos', 'masks']: (args.output/name).mkdir()
    rows, rejected, shape = [], [], None
    for file in sorted(args.photos.iterdir()):
        if file.suffix.lower() not in {'.jpg', '.jpeg', '.png'}: continue
        with Image.open(file) as image:
            if image.getexif().get(274, 1) != 1: raise ValueError('normalize EXIF orientation before Vision and crop')
            width, height = image.size
        if shape is not None and shape != (width, height): raise ValueError('mixed original frames need separate calibrated groups')
        shape = width, height
        if args.masks:
            path=args.masks/(file.name+'.png')
            if not path.is_file():path=args.masks/file.with_suffix('.png').name
            if not path.is_file():raise ValueError('missing own mask: '+file.name)
            candidates=[(str(path),cv2.imread(str(path),cv2.IMREAD_GRAYSCALE))]
        else:candidates = [(str(p), cv2.imread(str(p), cv2.IMREAD_GRAYSCALE)) for p in sorted(args.instances.glob(file.name+'.instance-*.png'))]
        if any(mask is None or mask.shape != (height, width) for _, mask in candidates): raise ValueError('invalid foreground frame')
        chosen = choose_instance(candidates,args.maximum_foreground_fraction)
        if chosen is None: rejected.append(file.name); continue
        _, maskfile, _, box = chosen; rows.append({'name': file.name, 'mask': maskfile, 'box': box})
    if len(rows) < 8: raise ValueError('fewer than eight bounded object masks; no geometry fabricated')
    suggested = common_crop([row['box'] for row in rows], width, height)
    crop = suggested if args.shared_crop else [0,0,width,height]; a, b, c, d = crop
    for row in rows:
        source=args.photos/row['name'];target=args.output/'photos'/row['name']
        if crop == [0,0,width,height]:
            if source.stat().st_dev==target.parent.stat().st_dev:os.link(source,target)
            else:shutil.copy2(source,target)
        else:
            rgb = cv2.imread(str(source))
            cv2.imwrite(str(target), rgb[b:d, a:c], [cv2.IMWRITE_JPEG_QUALITY, 97])
        mask = cv2.imread(row['mask'], cv2.IMREAD_GRAYSCALE)
        cv2.imwrite(str(args.output/'masks'/(row['name']+'.png')), np.uint8(mask[b:d, a:c] > 127)*255)
    calibration = json.loads(args.calibration.read_text()) if args.calibration else None
    lens = initial_lens(width, height, crop, calibration, args.fov)
    (args.output/'template.json').write_text(json.dumps({'names': [row['name'] for row in rows], 'lens': lens, 'keypoints': [], 'pairs': []})+'\n')
    (args.output/'report.json').write_text(json.dumps({'photos': len(rows), 'rejected': rejected, 'crop_xyxy': crop, 'suggested_crop_xyxy':suggested,'shared_crop':args.shared_crop,'original_size': [width, height], 'initial_lens': lens, 'manual_selections': False, 'reference_used': False}, indent=2)+'\n')


if __name__ == '__main__': main()
