"""Reference values for the native port: the MLX prototype's own model code (executed from neus_facto_mlx.py up to the
model's construction), with a small hash table and perturbed parameters, on fixed rays and samples. Saves the
parameters, the per-sample values, the loss and the gradient of every parameter.

usage: export_fixture.py PACK OUT.npz [TABLE_LOG2 (12)]   (PACK only provides a few photos; nothing of it is saved)"""
import os, sys
from pathlib import Path
import numpy as np

HERE = Path(__file__).resolve().parent
pack, target_path = sys.argv[1], Path(sys.argv[2]); table_log2 = int(sys.argv[3]) if len(sys.argv) > 3 else 12
os.environ.setdefault('VIEW_STEP', '100'); os.environ['HULL_BAND'] = '0'
whole = (HERE / 'neus_facto_mlx.py').read_text()
# Everything before the model is made and trained, plus the hull grid and band definitions that follow it.
source = whole[:whole.index('\nfield = Field()')] + whole[whole.index('\nHULL_GRID = None'):whole.index('\ndef fit_hull')]
scratch = target_path.parent / (target_path.stem + '-scratch'); sys.argv = ['neus_facto_mlx.py', pack, str(scratch)]
space = {'__name__': 'fixture'}
exec(compile(source, 'neus_facto_mlx.py', 'exec'), space)
mx, nn = space['mx'], space['nn']
from mlx.utils import tree_flatten
mx.set_default_device(mx.cpu)
space['TABLE'] = 1 << table_log2
space['OFFSETS'] = mx.array((np.arange(space['LEVELS']) * space['TABLE']).astype(np.uint32))

r = np.random.default_rng(7)
field = space['Field']()
# Perturb everything so that every term matters (a fresh table is near zero, a fresh network nearly a sphere).
field.table = mx.array((r.random(field.table.shape) * 2 - 1).astype(np.float32) * 0.05)
for layer in field.geometry + field.colour:
    layer.v = layer.v + mx.array(r.normal(0, 0.1, layer.v.shape).astype(np.float32)) * mx.sqrt((layer.v * layer.v).mean())
    layer.g = mx.array(r.uniform(0.5, 1.5, layer.g.shape).astype(np.float32)) * layer.g
    layer.b = layer.b + mx.array(r.normal(0, 0.1, layer.b.shape).astype(np.float32))
field.variance = mx.array(0.45)
# Move the surface outwards so that most rays cross it (the perturbed network is positive on every sample).
field.geometry[2].b = field.geometry[2].b - mx.array(np.eye(1, field.geometry[2].b.shape[0], 0, dtype=np.float32)[0] * 0.45)
mx.eval(field.parameters())

# Rays from a sphere of radius 2 towards points near the centre, 32 samples each between the unit sphere's ends.
n, count = 64, 32
o = r.normal(size=(n, 3)); o = 2.0 * o / np.linalg.norm(o, axis=1, keepdims=True)
d = r.normal(0, 0.15, (n, 3)) - o; d /= np.linalg.norm(d, axis=1, keepdims=True)
b = (o * d).sum(1); root = np.sqrt(np.maximum(b * b - ((o * o).sum(1) - 1), 1e-6)); near, far = -b - root, -b + root
edges = np.sort(near[:, None] + (far - near)[:, None] * np.concatenate([np.zeros((n, 1)), r.random((n, count - 1)), np.ones((n, 1))], 1), 1)
colour_target = r.random((n, 3)); mask = (r.random(n) < 0.6).astype(np.float64); anneal = 0.3
o, d, edges, colour_target, mask = (mx.array(a.astype(np.float32)) for a in (o, d, edges, colour_target, mask))

start = edges[:, :-1]
points = (o[:, None, :] + start[..., None] * d[:, None, :]).reshape(-1, 3)
directions = mx.broadcast_to(d[:, None, :], (n, count, 3)).reshape(-1, 3)
distance, features, gradient = field.with_gradient(points)
normal = gradient / mx.maximum(mx.sqrt((gradient * gradient).sum(-1, keepdims=True)), 1e-12)
rgb = field.shade(points, directions, normal, features)
colour, opacity, _ = space['render'](field, o, d, edges, anneal)
value_and_grad = nn.value_and_grad(field, lambda *a: space['loss_function'](field, *a))
(loss, parts), grads = value_and_grad(o, d, edges, colour_target, mask, anneal)

# The prototype's sampler on the same rays (no hull band): its one random draw, the stratified offset per ray, is
# repeated from the same seed and saved.
mx.random.seed(11); offsets = mx.random.uniform(shape=(n, 1))
mx.random.seed(11); sampled = space['sample_neus'](field, o, d)

# The same with the hull band (the prototype's default 0.05): its hull grid, and the band-limited samples.
space['HULL_BAND'] = 0.05
mx.random.seed(12); band_offsets = mx.random.uniform(shape=(n, 1))
mx.random.seed(12); band_sampled = space['sample_neus'](field, o, d)
space['HULL_BAND'] = 0.0

# Two training steps as the prototype takes them (the rays in two parts, gradients averaged, Adam), on fixed edges.
import mlx.optimizers as optim
from mlx.utils import tree_map
parameters_before = dict(tree_flatten(field.parameters()))
optimizer = optim.Adam(learning_rate=5e-4, eps=1e-15)
after = []
for step in range(2):
    total = None
    for part in range(2):
        s = slice(part * n // 2, (part + 1) * n // 2)
        (_, _), part_grads = value_and_grad(o[s], d[s], edges[s], colour_target[s], mask[s], anneal)
        part_grads = tree_map(lambda g: g / 2, part_grads)
        total = part_grads if total is None else tree_map(lambda a, b: a + b, total, part_grads)
    optimizer.update(field, total); mx.eval(field.parameters(), optimizer.state)
    after.append(dict(tree_flatten(field.parameters())))

arrays = {'band_offsets': band_offsets, 'band_edges': band_sampled, 'hull_grid': space['hull_grid'](), 'sample_offsets': offsets, 'sampled_edges': sampled, 'rays_origin': o, 'rays_direction': d, 'edges': edges, 'target': colour_target, 'mask': mask, 'anneal': mx.array(anneal),
          'points': points, 'distance': distance, 'features': features, 'gradient': gradient, 'rgb': rgb, 'colour': colour, 'opacity': opacity,
          'loss': loss, 'colour_loss': parts[0], 'eikonal': parts[1], 'mask_loss': parts[2], 'table_log2': mx.array(table_log2)}
arrays.update({f'param/{k}': v for k, v in parameters_before.items()})
arrays.update({f'grad/{k}': v for k, v in tree_flatten(grads)})
for step, values in enumerate(after):
    arrays.update({f'step{step + 1}/{k}': v for k, v in values.items()})
np.savez(target_path, **{k: np.array(v.astype(mx.int32) if k == 'table_log2' else v.astype(mx.float32)) for k, v in arrays.items()})
print(f'wrote {target_path}: loss {float(loss):.6f} (colour {float(parts[0]):.6f}, eikonal {float(parts[1]):.6f}, mask {float(parts[2]):.6f}), '
      f'{len(arrays)} arrays')
for k, v in sorted(arrays.items()):
    if k.startswith('grad/'):
        print(f'  {k}: |g| {float(mx.sqrt((v * v).sum())):.4e}')
