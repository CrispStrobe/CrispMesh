"""Where do the 48 samples of neus_facto_mlx.py fall relative to the network's own first surface crossing?
Small batches on purpose (shared machine). usage: diag_samples.py PACK RUN_DIR"""
import os, sys, runpy
from pathlib import Path
run = Path(sys.argv[2]); os.environ['STEPS'] = '0'; os.environ['MESH'] = '8'; os.environ['MEMORY_GIB'] = '1.5'
sys.argv = [sys.argv[0], sys.argv[1], str(run / 'diag')]
g = runpy.run_path(os.path.join(os.path.dirname(__file__), 'neus_facto_mlx.py'))
mx, np = g['mx'], g['np']; field = g['field']
saved = mx.load(str(run / 'checkpoint.safetensors')); step = int(saved.pop('step').item())
field.update(g['tree_unflatten'](list(saved.items()))); s = float(field.sharpness()); print('step', step, 'sharpness', s)
cache = g['cache_distances'](field)
o, d, target, mask = (mx.array(a) for a in g['rays'](512))
edges = np.array(g['sample'](field, cache, o, d, mx.array(s), g['FLOOR']))
b = (o * d).sum(-1); c = (o * o).sum(-1) - 1.0; root = mx.sqrt(mx.maximum(b * b - c, 1e-6))
near, far = mx.maximum(-b - root, 0.05), mx.maximum(-b + root, 0.06)
t_all = near[:, None] + (far - near)[:, None] * mx.linspace(0, 1, 2048)[None, :]
values = np.concatenate([np.array(field.distance((o[:, None, :] + t_all[:, k:k + 256, None] * d[:, None, :]).reshape(-1, 3)).reshape(-1, 256)) for k in range(0, 2048, 256)], 1)
t_all = np.array(t_all); print('distance range', values.min(), values.max())
mask = np.array(mask)
crossing = (values[:, :-1] > 0) & (values[:, 1:] <= 0); has = crossing.any(1)
tstar = t_all[np.arange(len(t_all)), crossing.argmax(1)][has]; e = edges[has]
print('rays with a crossing %.3f, foreground %.3f, both %.3f' % (has.mean(), mask.mean(), (has & (mask > 0)).mean()))
print('crossing inside the sampled range %.3f' % np.mean((tstar >= e[:, 0]) & (tstar <= e[:, -1])))
gap = np.abs(e - tstar[:, None]).min(1); near = (np.abs(e - tstar[:, None]) < 2 / s).sum(1)
print('nearest sample to the crossing: median %.5f p90 %.5f (1/s %.5f); samples within 2/s: median %d p10 %d'
      % (np.median(gap), np.percentile(gap, 90), 1 / s, np.median(near), np.percentile(near, 10)))
width = e[:, -1] - e[:, 0]; print('sampled range width: median %.4f p10 %.4f' % (np.median(width), np.percentile(width, 10)))
