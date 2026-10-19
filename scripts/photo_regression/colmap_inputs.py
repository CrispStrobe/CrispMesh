#!/usr/bin/env python
"""Build engine inputs from OWN photo-recovered COLMAP cameras and OWN RGB masks.

COLMAP and engine cameras both use raster-corner coordinates (first pixel centre
is .5). Subtract integer crop offsets only. Dataset-supplied evaluation cameras
and masks must never be used as reconstruction inputs.
"""
import argparse
import hashlib
import json
from pathlib import Path

import numpy as np
from PIL import Image


def native_intrinsics(model, parameters, pixel_scale=1., crop_origin=(0, 0)):
    p = np.asarray(parameters, dtype=float)
    if not np.isfinite(p).all() or not np.isfinite(pixel_scale) or pixel_scale <= 0:
        raise ValueError('camera parameters and positive pixel scale must be finite')
    if model in ('SIMPLE_PINHOLE', 'SIMPLE_RADIAL'):
        required = 3 if model == 'SIMPLE_PINHOLE' else 4
        if len(p) != required:
            raise ValueError('wrong camera parameter count')
        fx = fy = p[0]
        cx, cy = p[1:3]
        distortion = p[3:]
    elif model in ('PINHOLE', 'FULL_OPENCV'):
        required = 4 if model == 'PINHOLE' else 12
        if len(p) != required:
            raise ValueError('wrong camera parameter count')
        fx, fy, cx, cy = p[:4]
        distortion = p[4:]
    else:
        raise ValueError('this diagnostic adapter only accepts supported pinhole or zero-distortion cameras')
    if fx <= 0 or fy <= 0 or np.any(distortion != 0):
        raise ValueError('positive focal lengths and already undistorted RGB/masks are required')
    left, top = crop_origin
    if not np.isfinite([left, top]).all():
        raise ValueError('crop origin must be finite')
    # Neither conversion between these two formats nor an integer crop subtracts .5.
    return [float(fx * pixel_scale), float(fy * pixel_scale),
            float(cx * pixel_scale - left), float(cy * pixel_scale - top)]


def object_crop(mask, padding):
    if not np.isfinite(padding) or not 0 <= padding <= 1:
        raise ValueError('crop padding must be between 0 and 1')
    mask = np.asarray(mask)
    if mask.ndim != 2:
        raise ValueError('mask must be 2D')
    ys, xs = np.where(mask > 127)
    if not len(xs):
        raise ValueError('foreground mask is empty')
    pad = int(padding * max(xs.max() - xs.min() + 1, ys.max() - ys.min() + 1))
    return [max(0, int(xs.min()) - pad), max(0, int(ys.min()) - pad),
            min(mask.shape[1], int(xs.max()) + pad + 1), min(mask.shape[0], int(ys.max()) + pad + 1)]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--model', type=Path, required=True)
    parser.add_argument('--photos', type=Path, required=True)
    parser.add_argument('--masks', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True, help='fresh output directory')
    parser.add_argument('--model-pixel-scale', type=float, default=1., help='Only for a deliberately scaled diagnostic camera model')
    parser.add_argument('--crop-padding', type=float, default=.15, help='fraction of foreground maximum extent')
    parser.add_argument('--normalize-world', action='store_true', help='For arbitrary-scale own SfM only; preserves all image projections')
    args = parser.parse_args()
    import pycolmap

    if args.output.exists():
        parser.error('--output must be a fresh directory')
    if not np.isfinite(args.model_pixel_scale) or args.model_pixel_scale <= 0:
        parser.error('--model-pixel-scale must be positive and finite')
    model = pycolmap.Reconstruction(str(args.model))
    if not model.images or not model.points3D:
        parser.error('photo-recovered model must contain registered views and object landmarks')
    transform = model.normalize(extent=1., use_images=False).matrix().tolist() if args.normalize_world else None
    points = np.array([model.points3D[i].xyz for i in sorted(model.points3D)])
    if not np.isfinite(points).all():
        parser.error('nonfinite recovered object landmarks')
    rows, records = [], []
    # Validate every association and camera before creating an output folder.
    images = sorted(model.images.values(), key=lambda image: image.name)
    for image in images:
        camera = model.cameras[image.camera_id]
        native_intrinsics(camera.model.name, camera.params, args.model_pixel_scale)
        photo = args.photos / image.name
        candidates = [args.masks / (image.name + '.png'), args.masks / Path(image.name).with_suffix('.png'), args.masks / image.name]
        mask_path = next((p for p in candidates if p.is_file()), None)
        if not photo.is_file() or mask_path is None:
            parser.error(f'photo or own mask missing for {image.name}')
        with Image.open(photo) as rgb, Image.open(mask_path) as mask:
            if rgb.getexif().get(274, 1) != 1:
                parser.error('normalize EXIF orientation before this diagnostic adapter')
            expected = np.array([camera.width, camera.height]) * args.model_pixel_scale
            if rgb.size != mask.size or not np.allclose(expected, rgb.size, rtol=0, atol=1e-6):
                parser.error(f'camera/photo/mask dimensions disagree for {image.name}')
            box = object_crop(np.array(mask.convert('L')), args.crop_padding)
        records.append((image, camera, photo, mask_path, box))
    args.output.mkdir(parents=True)
    (args.output / 'images').mkdir()
    (args.output / 'masks').mkdir()
    provenance = []
    for index, (image, camera, photo, mask_path, box) in enumerate(records):
        name = f'view_{index:04}'
        with Image.open(photo) as rgb, Image.open(mask_path) as mask:
            rgb.convert('RGB').crop(box).save(args.output / 'images' / (name + '.png'))
            binary = (np.array(mask.convert('L')) > 127).astype('uint8') * 255
            Image.fromarray(binary).crop(box).save(args.output / 'masks' / (name + '.png'))
            width, height = rgb.size
        rows.append({'name': name, 'source': image.name, 'image': f'images/{name}.png', 'mask': f'masks/{name}.png',
                     'width': box[2] - box[0], 'height': box[3] - box[1],
                     'k': native_intrinsics(camera.model.name, camera.params, args.model_pixel_scale, box[:2]),
                     'rotation': image.cam_from_world.rotation.matrix().tolist(), 'translation': image.cam_from_world.translation.tolist(),
                     'image_crop': {'source_width': width, 'source_height': height, 'xyxy': box}})
        provenance.append({'photo': image.name, 'photo_sha256': hashlib.sha256(photo.read_bytes()).hexdigest(),
                           'mask_sha256': hashlib.sha256(mask_path.read_bytes()).hexdigest(), 'crop': box})
    np.save(args.output / 'sparse_points.npy', points, allow_pickle=False)
    (args.output / 'cameras.json').write_text(json.dumps({'views': rows}, indent=2) + '\n')
    (args.output / 'provenance.json').write_text(json.dumps({'method': 'Own photo-recovered COLMAP to native inputs',
        'pixel_convention': 'Both formats use .5-centred pixels; scale corner coordinates then subtract integer crop offsets',
        'model_pixel_scale': args.model_pixel_scale, 'crop_padding': args.crop_padding,
        'world_similarity': transform, 'photos': provenance,
        'limits': 'Own photo-derived poses/masks only; no geometry-accuracy claim. Zero-distortion models required.'}, indent=2) + '\n')
    print(json.dumps({'views': len(rows), 'landmarks': len(points), 'inputs': str(args.output)}))


if __name__ == '__main__':
    main()
