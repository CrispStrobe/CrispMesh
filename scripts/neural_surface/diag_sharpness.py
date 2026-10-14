"""Loss of a checkpoint of neus_facto_mlx.py over fixed rays at several sharpness values, geometry held fixed.
usage: diag_sharpness.py PACK RUN_DIR"""
import os, sys, runpy, math
from pathlib import Path
run = Path(sys.argv[2]); os.environ['STEPS'] = '0'; os.environ['MESH'] = '8'; os.environ['MEMORY_GIB'] = '1.5'
sys.argv = [sys.argv[0], sys.argv[1], str(run / 'diag')]
g = runpy.run_path(os.path.join(os.path.dirname(__file__), 'neus_facto_mlx.py'))
mx, np = g['mx'], g['np']; field = g['field']
saved = mx.load(str(run / 'checkpoint.safetensors')); step = int(saved.pop('step').item())
field.update(g['tree_unflatten'](list(saved.items()))); print('step', step, 'learned sharpness', float(field.sharpness()))
cache = g['cache_distances'](field)
batches = [tuple(mx.array(a) for a in g['rays'](512)) for _ in range(4)]
for s in (20, 35, 70, 140, 280):
    field.variance = mx.array(math.log(s) / 10.0); rows = []
    for o, d, target, mask in batches:
        mx.random.seed(1)
        edges = g['sample'](field, cache, o, d, field.sharpness(), g['FLOOR'])
        loss, parts = g['loss_function'](field, o, d, edges, target, mask, min(1.0, step / 50000))
        rows.append([float(loss)] + [float(x) for x in parts])
    r = np.mean(rows, 0); print('sharpness %4d: loss %.4f colour %.4f eikonal %.4f mask %.4f' % (s, *r), flush=True)
