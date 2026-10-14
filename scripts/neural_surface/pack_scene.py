"""Package a recovered scene for external surface methods: colour photos cropped to one common window around each
mask (reduced by DIV, JPEG), masks (PNG), cameras (OpenCV convention, intrinsics of the crop), a bounding sphere of
the object and surface samples of our silhouette mesh for initialisation. No scan, no supplied poses.
usage: pack_scene.py INPUTS RGB_DIR MASK_DIR HULL_MESH.stl OUT DIV"""
import json, sys
from pathlib import Path
import numpy as np, cv2
sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from scripts.turntable_mesh.scan_evaluate import read_binary_stl, sample_surface
inputs, rgb, masks, mesh, out, div = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3]), Path(sys.argv[4]), Path(sys.argv[5]), int(sys.argv[6])
(out / 'images').mkdir(parents=True, exist_ok=True); (out / 'masks').mkdir(exist_ok=True)
views = json.loads((inputs / 'cameras.json').read_text())['views']
boxes = []
for v in views:
    m = cv2.imread(str(masks / (v['name'] + '.png')), 0) > 127; ys, xs = np.nonzero(m); boxes.append((xs.min(), ys.min(), xs.max() + 1, ys.max() + 1))
boxes = np.array(boxes); margin = 48
cw = int(np.ceil(((boxes[:, 2] - boxes[:, 0]).max() + 2 * margin) / (16 * div)) * 16 * div); ch = int(np.ceil(((boxes[:, 3] - boxes[:, 1]).max() + 2 * margin) / (16 * div)) * 16 * div)
frames = []
for v, b in zip(views, boxes):
    name = v['name']; image = cv2.imread(str(rgb / (name + '.png')), cv2.IMREAD_COLOR); mask = cv2.imread(str(masks / (name + '.png')), 0)
    H, W = mask.shape; x0 = int(np.clip((b[0] + b[2]) // 2 - cw // 2, 0, W - cw)); y0 = int(np.clip((b[1] + b[3]) // 2 - ch // 2, 0, H - ch))
    size = (cw // div, ch // div)
    cv2.imwrite(str(out / 'images' / (name + '.jpg')), cv2.resize(image[y0:y0 + ch, x0:x0 + cw], size, interpolation=cv2.INTER_AREA), [cv2.IMWRITE_JPEG_QUALITY, 96])
    cv2.imwrite(str(out / 'masks' / (name + '.png')), ((cv2.resize(mask[y0:y0 + ch, x0:x0 + cw], size, interpolation=cv2.INTER_AREA) >= 128) * 255).astype('uint8'))
    fx, fy, cx, cy = v['k']
    frames.append({'name': name, 'image': f'images/{name}.jpg', 'mask': f'masks/{name}.png', 'w': size[0], 'h': size[1],
                   'fx': fx / div, 'fy': fy / div, 'cx': (cx - x0) / div, 'cy': (cy - y0) / div,       # pixel centres at integer + 0.5
                   'rotation': v['rotation'], 'translation': v['translation']})
tri = read_binary_stl(mesh).astype(float); pts, normals = sample_surface(tri, 100000, np.random.default_rng(0))
centre = 0.5 * (pts.min(0) + pts.max(0)); radius = float(np.linalg.norm(pts - centre, axis=1).max())
np.save(out / 'hull_points.npy', pts.astype(np.float32)); np.save(out / 'hull_normals.npy', normals.astype(np.float32))
(out / 'scene.json').write_text(json.dumps({'convention': 'camera = rotation @ world + translation; x right, y down, z forward; pixel x = fx * X / Z + cx with pixel centres at integer + 0.5',
                                             'centre': centre.tolist(), 'radius': radius, 'frames': frames}, indent=1))
print(len(frames), 'views', size, 'centre', centre.round(4), 'radius', round(radius, 4))
