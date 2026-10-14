"""NeuS-facto on MLX: a port of the model we ran as the reference (hummat/sdfstudio 12a3f75, Apache-2.0, method
`neus-facto` with `inside-outside False`, `fg-mask-loss-mult 0.1`, black background), written from its configuration.
Research prototype; the pipeline itself must not depend on Python.

Same as the reference: 16-level hash grid (16 .. 2048 over [-2, 2], 2^19 entries, 2 features, linear interpolation),
position frequencies (6), geometry network 2 x 256 softplus(100) with weight normalisation, 256 geometry features,
colour network 2 x 256 on (position, direction frequencies (4), normal, features), NeuS opacities with the cosine
annealing (50 000 steps) and a learned sharpness (starting at exp(3)), analytic gradients, L1 colour against the photo
with a black background, eikonal 0.1, mask cross-entropy 0.1, Adam 5e-4 with 500 warm-up steps and a cosine to 5 %.

Different (each a switch; the defaults are the setup that matched the reference best so far):
  HULL=600     start: the geometry network is first fitted to our silhouette hull's distance (0: sphere of radius 0.5)
  SAMPLER=neus samples per ray as in the original NeuS (64 even, four rounds of 16 at sharpness 64..512, all 128
               rendered); `cache` = own two-round sampler from a coarse distance grid (held sharpness at the grid's
               resolution: rejected)
  RAYS=1024 PARTS=4  rays per step, processed in parts to bound memory (reference: 2048 rays, 48 samples each)
  GRADIENT=separate  value and input gradient in two passes; `vjp` = one pass
  MEMORY_GIB=2  MLX memory ceiling; buffer cache 256 MB

usage: neus_facto_mlx.py PACK OUT_DIR   (PACK from pack_scene.py)
env: STEPS (20000) SNAPSHOT (0: none; else mesh + checkpoint every N steps; a run resumes from its checkpoint)
     MESH (512) VIEW_STEP (1) SEED (0)"""
import json, math, os, sys, time
from pathlib import Path
import numpy as np, cv2
import mlx.core as mx
import mlx.nn as nn
import mlx.optimizers as optim
from mlx.utils import tree_flatten, tree_map
from skimage import measure
import shutil
from mlx.utils import tree_unflatten

pack, out = Path(sys.argv[1]), Path(sys.argv[2]); out.mkdir(parents=True, exist_ok=True)
STEPS = int(os.environ.get('STEPS', 20000)); RAYS = int(os.environ.get('RAYS', 1024)); PARTS = int(os.environ.get('PARTS', 4)); VIEW_STEP = int(os.environ.get('VIEW_STEP', 1))
HULL = int(os.environ.get('HULL', 600)); SAMPLER = os.environ.get('SAMPLER', 'neus'); GRADIENT = os.environ.get('GRADIENT', 'separate'); FLOOR = float(os.environ.get('FLOOR', 0.05)); FLOOR_STEPS = int(os.environ.get('FLOOR_STEPS', 0))
MESH = int(os.environ.get('MESH', 512)); SEED = int(os.environ.get('SEED', 0)); SNAPSHOT = int(os.environ.get('SNAPSHOT', 0))
LEVELS, TABLE, FEATURES, BASE, FINEST = 16, 1 << 19, 2, 16, 2048
HIDDEN, GEO, SAMPLES, COARSE, MIDDLE, CACHE = 256, 256, 48, 128, 40, 128
EIKONAL, MASK, ANNEAL_END, WARM_UP, LR, LR_END = 0.1, 0.1, 50000, 500, 5e-4, 0.05
rng = np.random.default_rng(SEED); mx.random.seed(SEED)
# Keep the footprint small on a shared 16 GB machine: no large buffer cache, and a hard ceiling on GPU memory.
mx.set_cache_limit(256 * 2 ** 20); mx.set_memory_limit(int(float(os.environ.get('MEMORY_GIB', 2.0)) * 2 ** 30))
T0 = time.time()


def log(*parts):
    print(f'[{time.time() - T0:6.0f}s]', *parts, flush=True)


# --- data: photos with the background set to black, as the reference was fed
scene = json.loads((pack / 'scene.json').read_text()); frames = scene['frames'][::VIEW_STEP]
centre = np.array(scene['centre']); unit = 1.0 / (1.15 * scene['radius'])
masks = np.stack([cv2.imread(str(pack / f['mask']), 0) > 127 for f in frames])
images = np.stack([cv2.cvtColor(cv2.imread(str(pack / f['image'])), cv2.COLOR_BGR2RGB) for f in frames]); images[~masks] = 0
V, H, W = masks.shape
rotation = np.array([f['rotation'] for f in frames]); translation = np.array([f['translation'] for f in frames])
origin = (-np.einsum('vji,vj->vi', rotation, translation) - centre) * unit
intrinsics = np.array([[f['fx'], f['fy'], f['cx'], f['cy']] for f in frames])
log(f'{V} views {W}x{H}, foreground {masks.mean():.3f}')


def rays(count):
    v, y, x = rng.integers(V, size=count), rng.integers(H, size=count), rng.integers(W, size=count)
    fx, fy, cx, cy = intrinsics[v].T
    d = np.stack([(x + 0.5 - cx) / fx, (y + 0.5 - cy) / fy, np.ones(count)], 1)
    d = np.einsum('nji,nj->ni', rotation[v], d); d /= np.linalg.norm(d, axis=1, keepdims=True)
    return origin[v].astype(np.float32), d.astype(np.float32), (images[v, y, x] / 255.0).astype(np.float32), masks[v, y, x].astype(np.float32)


# --- model
SCALINGS = mx.array(np.floor(BASE * np.exp((np.log(FINEST) - np.log(BASE)) / (LEVELS - 1)) ** np.arange(LEVELS)).astype(np.float32))
OFFSETS = mx.array((np.arange(LEVELS) * TABLE).astype(np.uint32))
POSITION_FREQUENCIES = mx.array(2.0 ** np.arange(6, dtype=np.float32)); DIRECTION_FREQUENCIES = mx.array(2.0 ** np.arange(4, dtype=np.float32))


def frequencies(x, freqs):
    scaled = (x[..., None] * freqs).reshape(*x.shape[:-1], -1)
    return mx.sin(mx.concatenate([scaled, scaled + math.pi / 2], -1))


def hash_features(table, p):
    """p [n, 3] in [0, 1] -> [n, LEVELS * FEATURES], trilinear over the eight hashed corners of every level."""
    scaled = p[:, None, :] * SCALINGS[None, :, None]                        # [n, L, 3]
    low = mx.floor(scaled); f = scaled - low; low = mx.stop_gradient(low).astype(mx.uint32)
    total = 0
    for a in (0, 1):
        for b in (0, 1):
            for c in (0, 1):
                x, y, z = low[..., 0] + a, low[..., 1] + b, low[..., 2] + c
                index = (mx.bitwise_xor(mx.bitwise_xor(x, y * mx.array(2654435761, mx.uint32)), z * mx.array(805459861, mx.uint32)) & (TABLE - 1)) + OFFSETS[None, :]
                w = (f[..., 0] if a else 1 - f[..., 0]) * (f[..., 1] if b else 1 - f[..., 1]) * (f[..., 2] if c else 1 - f[..., 2])
                total = total + table[index] * w[..., None]
    return total.reshape(p.shape[0], LEVELS * FEATURES)


class Normalised(nn.Module):
    """A linear layer with weight normalisation: weight = g * v / |v| per output."""

    def __init__(self, weight, bias):
        super().__init__()
        self.v = mx.array(weight.astype(np.float32)); self.g = mx.array(np.linalg.norm(weight, axis=1).astype(np.float32)); self.b = mx.array(bias.astype(np.float32))

    def __call__(self, x):
        w = self.v * (self.g / mx.sqrt((self.v * self.v).sum(1)))[:, None]
        return x @ w.T + self.b


def softplus(x):
    return mx.logaddexp(x * 100, 0.0) * 0.01


class Field(nn.Module):
    def __init__(self):
        super().__init__()
        r = np.random.default_rng(SEED)
        self.table = mx.array(((r.random((TABLE * LEVELS, FEATURES)) * 2 - 1) * 1e-3).astype(np.float32))
        n_in = 3 + 36 + LEVELS * FEATURES
        first = np.zeros((HIDDEN, n_in)); first[:, :3] = r.normal(0, math.sqrt(2) / math.sqrt(HIDDEN), (HIDDEN, 3))
        second = r.normal(0, math.sqrt(2) / math.sqrt(HIDDEN), (HIDDEN, HIDDEN))
        last = r.normal(math.sqrt(math.pi) / math.sqrt(HIDDEN), 1e-4, (1 + GEO, HIDDEN))
        self.geometry = [Normalised(first, np.zeros(HIDDEN)), Normalised(second, np.zeros(HIDDEN)), Normalised(last, np.full(1 + GEO, -0.5))]
        n_colour = 3 + 27 + 3 + GEO
        def kaiming(n_out, n_in_):
            bound = math.sqrt(6 / n_in_); return r.uniform(-bound, bound, (n_out, n_in_))
        self.colour = [Normalised(kaiming(HIDDEN, n_colour), np.zeros(HIDDEN)), Normalised(kaiming(HIDDEN, HIDDEN), np.zeros(HIDDEN)), Normalised(kaiming(3, HIDDEN), np.zeros(3))]
        self.variance = mx.array(float(os.environ.get('VARIANCE', 0.3)))

    def geometry_net(self, p):
        x = mx.concatenate([p, frequencies(p, POSITION_FREQUENCIES), hash_features(self.table, (p + 2.0) / 4.0)], -1)
        x = softplus(self.geometry[0](x)); x = softplus(self.geometry[1](x)); x = self.geometry[2](x)
        return x[:, 0], x[:, 1:]

    def distance(self, p):
        return self.geometry_net(p)[0]

    def with_gradient(self, p):
        """Distance, geometry features and the distance's gradient at p. GRADIENT=separate (default): a forward pass and
        a separate gradient pass. GRADIENT=vjp: both from one vector-Jacobian product. (mx.value_and_grad with extra
        outputs is not usable here: gradients through its extra outputs are dropped, see mlx_aux_repro.py.)"""
        if GRADIENT == 'vjp':
            (distance, features), (gradient,) = mx.vjp(lambda q: list(self.geometry_net(q)), [p], [mx.ones((p.shape[0],)), mx.zeros((p.shape[0], GEO))])
            return distance, features, gradient
        distance, features = self.geometry_net(p)
        return distance, features, mx.grad(lambda q: self.geometry_net(q)[0].sum())(p)

    def shade(self, p, direction, normal, features):
        x = mx.concatenate([p, direction, frequencies(direction, DIRECTION_FREQUENCIES), normal, features], -1)
        x = nn.relu(self.colour[0](x)); x = nn.relu(self.colour[1](x))
        return mx.sigmoid(self.colour[2](x)) * 1.002 - 0.001

    def sharpness(self):
        return mx.clip(mx.exp(self.variance * 10.0), 1e-6, 1e6)


def neus_alpha(distance, cosine, delta, sharpness, anneal):
    cosine = -(nn.relu(-cosine * 0.5 + 0.5) * (1.0 - anneal) + nn.relu(-cosine) * anneal)
    previous, following = mx.sigmoid((distance - cosine * delta * 0.5) * sharpness), mx.sigmoid((distance + cosine * delta * 0.5) * sharpness)
    return mx.clip((previous - following + 1e-5) / (previous + 1e-5), 0.0, 1.0)


def edges_from(edges, weights, count, floor):
    """`count` + 1 increasing edges drawn from a histogram (edges [n, k + 1], weights [n, k]) mixed with a uniform floor."""
    pdf = weights + 1e-5; pdf = pdf / pdf.sum(1, keepdims=True); pdf = (1 - floor) * pdf + floor / pdf.shape[1]
    cdf = mx.concatenate([mx.zeros((pdf.shape[0], 1)), mx.cumsum(pdf, axis=1)], 1)
    u = (mx.arange(count + 1) + mx.random.uniform(shape=(pdf.shape[0], 1))) / (count + 1)
    index = mx.clip((u[:, :, None] >= cdf[:, None, :]).sum(-1) - 1, 0, pdf.shape[1] - 1)
    c0, c1 = mx.take_along_axis(cdf, index, axis=1), mx.take_along_axis(cdf, index + 1, axis=1)
    e0, e1 = mx.take_along_axis(edges, index, axis=1), mx.take_along_axis(edges, index + 1, axis=1)
    return e0 + (e1 - e0) * mx.clip((u - c0) / mx.maximum(c1 - c0, 1e-8), 0.0, 1.0)


def trilinear(grid, p):
    r = grid.shape[0]; flat = grid.reshape(-1)
    g = mx.clip((p + 1.0) * (0.5 * (r - 1)), 0.0, r - 1.001)
    i = mx.floor(g); f = g - i; i = i.astype(mx.int32)
    ix, iy, iz = i[..., 0], i[..., 1], i[..., 2]
    total = 0
    for a in (0, 1):
        for b in (0, 1):
            for c in (0, 1):
                w = (f[..., 0] if a else 1 - f[..., 0]) * (f[..., 1] if b else 1 - f[..., 1]) * (f[..., 2] if c else 1 - f[..., 2])
                total = total + flat[((ix + a) * r + (iy + b)) * r + (iz + c)] * w
    return total


def cache_distances(field):
    """The surface's distances on a coarse grid: where to put the samples of the next steps (in place of proposal networks)."""
    axis = mx.linspace(-1, 1, CACHE); planes = []
    for n in range(CACHE):
        x, y, z = mx.meshgrid(axis[n:n + 1], axis, axis, indexing='ij')
        planes.append(field.distance(mx.stack([x, y, z], -1).reshape(-1, 3)).reshape(CACHE, CACHE)); mx.eval(planes[-1])
    return mx.stop_gradient(mx.stack(planes))


def interval_weights(value, sharpness):
    previous, following = mx.sigmoid(value[:, :-1] * sharpness), mx.sigmoid(value[:, 1:] * sharpness)
    alpha = mx.clip((previous - following + 1e-5) / (previous + 1e-5), 0.0, 1.0)
    return alpha * mx.cumprod(mx.concatenate([mx.ones((value.shape[0], 1)), 1.0 - alpha + 1e-7], 1), axis=1)[:, :-1]


def sample(field, cache, o, d, sharpness, floor):
    """Two rounds, as the reference's two proposal rounds: a wide band from the cached grid (which is only as exact as
    its cells, so it is read with a sharpness of half a cell at most), then the network's own distances inside it."""
    b = (o * d).sum(-1); c = (o * o).sum(-1) - 1.0; root = mx.sqrt(mx.maximum(b * b - c, 1e-6))
    near, far = mx.maximum(-b - root, 0.05), mx.maximum(-b + root, 0.06)
    edges = near[:, None] + (far - near)[:, None] * mx.linspace(0, 1, COARSE + 1)[None, :]
    value = trilinear(cache, o[:, None, :] + edges[..., None] * d[:, None, :])
    middle = edges_from(edges, interval_weights(value, mx.minimum(sharpness, 0.25 * CACHE)), MIDDLE, max(floor, 0.1))
    value = field.distance((o[:, None, :] + middle[..., None] * d[:, None, :]).reshape(-1, 3)).reshape(o.shape[0], MIDDLE + 1)
    return mx.stop_gradient(edges_from(middle, interval_weights(value, sharpness), SAMPLES, floor))


def sample_neus(field, o, d):
    """The original NeuS sampler: 64 even samples, then four rounds of 16 more where the surface's distances put
    weight, at fixed sharpness 64, 128, 256, 512. Returns 129 edges (the 128 samples and the far end)."""
    b = (o * d).sum(-1); c = (o * o).sum(-1) - 1.0; root = mx.sqrt(mx.maximum(b * b - c, 1e-6))
    near, far = mx.maximum(-b - root, 0.05), mx.maximum(-b + root, 0.06)
    n = o.shape[0]
    z = near[:, None] + (far - near)[:, None] * (mx.arange(64) + mx.random.uniform(shape=(n, 1))) / 64
    value = field.distance((o[:, None, :] + z[..., None] * d[:, None, :]).reshape(-1, 3)).reshape(n, -1)
    for i in range(4):
        mid = 0.5 * (value[:, :-1] + value[:, 1:]); dist = z[:, 1:] - z[:, :-1]
        cos = (value[:, 1:] - value[:, :-1]) / (dist + 1e-5)
        cos = mx.clip(mx.minimum(mx.concatenate([mx.zeros((n, 1)), cos[:, :-1]], 1), cos), -1e3, 0.0)
        inv_s = 64.0 * 2 ** i
        previous, following = mx.sigmoid((mid - cos * dist * 0.5) * inv_s), mx.sigmoid((mid + cos * dist * 0.5) * inv_s)
        alpha = (previous - following + 1e-5) / (previous + 1e-5)
        weights = alpha * mx.cumprod(mx.concatenate([mx.ones((n, 1)), 1.0 - alpha + 1e-7], 1), axis=1)[:, :-1]
        pdf = (weights + 1e-5) / (weights + 1e-5).sum(1, keepdims=True)
        cdf = mx.concatenate([mx.zeros((n, 1)), mx.cumsum(pdf, axis=1)], 1)
        u = mx.broadcast_to(mx.linspace(1 / 32, 1 - 1 / 32, 16)[None, :], (n, 16))
        index = mx.clip((u[:, :, None] >= cdf[:, None, :]).sum(-1) - 1, 0, pdf.shape[1] - 1)
        c0, c1 = mx.take_along_axis(cdf, index, axis=1), mx.take_along_axis(cdf, index + 1, axis=1)
        z0, z1 = mx.take_along_axis(z, index, axis=1), mx.take_along_axis(z, index + 1, axis=1)
        new = z0 + (z1 - z0) * mx.clip((u - c0) / mx.maximum(c1 - c0, 1e-8), 0.0, 1.0)
        new_value = field.distance((o[:, None, :] + new[..., None] * d[:, None, :]).reshape(-1, 3)).reshape(n, -1)
        z = mx.concatenate([z, new], 1); value = mx.concatenate([value, new_value], 1)
        order = mx.argsort(z, axis=1); z = mx.take_along_axis(z, order, axis=1); value = mx.take_along_axis(value, order, axis=1)
    return mx.stop_gradient(mx.concatenate([z, far[:, None]], 1))


def render(field, o, d, edges, anneal):
    n, count = o.shape[0], edges.shape[1] - 1
    start, delta = edges[:, :-1], edges[:, 1:] - edges[:, :-1]
    p = (o[:, None, :] + start[..., None] * d[:, None, :]).reshape(-1, 3)
    directions = mx.broadcast_to(d[:, None, :], (n, count, 3)).reshape(-1, 3)
    distance, features, gradient = field.with_gradient(p)
    alpha = neus_alpha(distance, (directions * gradient).sum(-1), delta.reshape(-1), field.sharpness(), anneal).reshape(n, count)
    transmittance = mx.cumprod(mx.concatenate([mx.ones((n, 1)), 1.0 - alpha + 1e-7], 1), axis=1)[:, :-1]
    weights = alpha * transmittance
    normal = gradient / mx.maximum(mx.sqrt((gradient * gradient).sum(-1, keepdims=True)), 1e-12)
    rgb = field.shade(p, directions, normal, features).reshape(n, count, 3)
    return (weights[..., None] * rgb).sum(1), weights.sum(1), gradient


def loss_function(field, o, d, edges, target, mask, anneal):
    colour, opacity, gradient = render(field, o, d, edges, anneal)
    colour_loss = mx.abs(colour - target).mean()
    eikonal = ((mx.sqrt((gradient * gradient).sum(-1) + 1e-12) - 1.0) ** 2).mean()
    opacity = mx.clip(opacity, 1e-3, 1.0 - 1e-3)
    mask_loss = -(mask * mx.log(opacity) + (1 - mask) * mx.log(1 - opacity)).mean()
    return colour_loss + EIKONAL * eikonal + MASK * mask_loss, (colour_loss, eikonal, mask_loss)


def save_mesh(field, path, resolution):
    axis = np.linspace(-1, 1, resolution, dtype=np.float32); values = np.zeros((resolution,) * 3, np.float32)
    for n in range(resolution):
        plane = np.stack(np.meshgrid(axis[n:n + 1], axis, axis, indexing='ij'), -1).reshape(-1, 3)
        values[n] = np.array(field.distance(mx.array(plane))).reshape(resolution, resolution)
    if not (values.min() < 0 < values.max()):
        return 0
    vertices, faces, _, _ = measure.marching_cubes(values, 0.0, spacing=(2.0 / (resolution - 1),) * 3)
    tri = ((vertices - 1.0) / unit + centre)[faces]
    normal = np.cross(tri[:, 1] - tri[:, 0], tri[:, 2] - tri[:, 0]); normal /= np.linalg.norm(normal, axis=1, keepdims=True) + 1e-12
    rec = np.zeros(len(tri), dtype=[('n', '<f4', 3), ('v', '<f4', (3, 3)), ('a', '<u2')]); rec['n'] = normal; rec['v'] = tri
    Path(path).write_bytes(b'crispmesh neus-facto (mlx)'.ljust(80, b' ') + np.uint32(len(tri)).tobytes() + rec.tobytes())
    return len(tri)


field = Field(); mx.eval(field.parameters())
log(f'{sum(v.size for _, v in tree_flatten(field.parameters())) / 1e6:.1f} M parameters')


def fit_hull(field, steps):
    """Start on our silhouette hull instead of a sphere: the geometry network is fitted to the hull's signed distance."""
    from scipy.spatial import cKDTree
    from scipy.ndimage import gaussian_filter
    points = (np.load(pack / 'hull_points.npy').astype(np.float64) - centre) * unit; normals = np.load(pack / 'hull_normals.npy').astype(np.float64)
    r = 160; axis = np.linspace(-1, 1, r); nodes = np.stack(np.meshgrid(axis, axis, axis, indexing='ij'), -1).reshape(-1, 3)
    distance, nearest = cKDTree(points).query(nodes, workers=-1)
    sign = np.sign(np.einsum('ij,ij->i', nodes - points[nearest], normals[nearest]))
    grid = mx.array(gaussian_filter((distance * np.where(sign == 0, 1, sign)).reshape(r, r, r), 0.7).astype(np.float32))
    fitter = optim.Adam(learning_rate=1e-3, eps=1e-15)

    def loss(p, target):
        distance, _, gradient = field.with_gradient(p)
        return mx.abs(distance - target).mean() + 0.1 * ((mx.sqrt((gradient * gradient).sum(-1) + 1e-12) - 1.0) ** 2).mean()
    step_fn = nn.value_and_grad(field, loss)
    for n in range(steps):
        near = mx.array(points[rng.integers(len(points), size=32768)].astype(np.float32)) + mx.random.normal((32768, 3)) * 0.03
        p = mx.clip(mx.concatenate([near, mx.random.uniform(-1, 1, (32768, 3))]), -1, 1)
        value, grads = step_fn(p, trilinear(grid, p)); fitter.update(field, grads); mx.eval(field.parameters(), fitter.state, value)
        if n % 200 == 0 or n == steps - 1:
            log(f'hull fit {n}: {float(value):.5f}')


optimizer = optim.Adam(learning_rate=LR, eps=1e-15)
value_and_grad = nn.value_and_grad(field, lambda *a: loss_function(field, *a))
history = []; first_step = 0
if (out / 'checkpoint.safetensors').exists():
    saved = mx.load(str(out / 'checkpoint.safetensors')); first_step = int(saved.pop('step').item()) + 1
    field.update(tree_unflatten(list(saved.items()))); mx.eval(field.parameters()); log(f'resumed after step {first_step - 1} (optimiser moments restart)')
if HULL and first_step == 0:
    fit_hull(field, HULL); log(f'hull mesh: {save_mesh(field, out / "mesh-hull.stl", 256)} triangles')
for step in range(first_step, STEPS):
    factor = step / WARM_UP if step < WARM_UP else (math.cos(math.pi * (step - WARM_UP) / (STEPS - WARM_UP)) + 1.0) * 0.5 * (1 - LR_END) + LR_END
    optimizer.learning_rate = LR * factor
    anneal = min(1.0, step / ANNEAL_END)
    if SAMPLER != 'neus' and (step % (50 if step < 3000 else 100) == 0 or step == first_step):
        cache = cache_distances(field)
    floor = FLOOR + (1.0 - FLOOR) * max(0.0, 1.0 - step / FLOOR_STEPS) if FLOOR_STEPS else FLOOR
    # The batch of RAYS rays is processed in PARTS pieces and the gradients averaged: the same step, less memory.
    total = None
    for _ in range(PARTS):
        o, d, target, mask = (mx.array(a) for a in rays(RAYS // PARTS))
        edges = sample_neus(field, o, d) if SAMPLER == 'neus' else sample(field, cache, o, d, field.sharpness(), floor)
        (loss, parts), grads = value_and_grad(o, d, edges, target, mask, anneal)
        grads = tree_map(lambda g: g / PARTS, grads)
        total = grads if total is None else tree_map(lambda a, b: a + b, total, grads)
        mx.eval(total, loss)
    optimizer.update(field, total)
    mx.eval(field.parameters(), optimizer.state, loss)
    if step % 100 == 0 or step == STEPS - 1:
        row = [float(loss)] + [float(x) for x in parts] + [float(field.sharpness())]; history.append([step] + row)
        log(f'step {step}: loss {row[0]:.4f} colour {row[1]:.4f} eikonal {row[2]:.4f} mask {row[3]:.4f} sharpness {row[4]:.0f} peak {mx.get_peak_memory() / 2 ** 30:.1f} GiB')
    low_disk = step % 100 == 0 and shutil.disk_usage('/System/Volumes/Data').free < 8.8 * 2 ** 30
    if (SNAPSHOT and step and step % SNAPSHOT == 0) or low_disk:
        mx.save_safetensors(str(out / 'checkpoint.safetensors'), dict(tree_flatten(field.parameters()), step=mx.array(step)))
        log(f'snapshot: {save_mesh(field, out / f"mesh-{step}.stl", 256)} triangles')
    if low_disk:
        log('stopped: less than 8.8 GiB free on the disk; run again to resume'); sys.exit(3)
train_seconds = time.time() - T0
triangles = save_mesh(field, out / 'mesh.stl', MESH)
(out / 'summary.json').write_text(json.dumps({'steps': STEPS, 'rays': RAYS, 'views': V, 'train_seconds': train_seconds, 'seconds': time.time() - T0,
                                               'triangles': triangles, 'peak_memory_gib': mx.get_peak_memory() / 2 ** 30, 'history': history}, indent=1))
log(f'done: {triangles} triangles, training {train_seconds:.0f}s')
