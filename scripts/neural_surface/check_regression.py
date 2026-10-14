"""Checks of neus_facto_mlx.py: does a plain regression of the geometry network converge, with and without the nested gradient?"""
import os, sys, runpy
os.environ['STEPS'] = '0'; os.environ['MESH'] = '8'
g = runpy.run_path(os.path.join(os.path.dirname(__file__), 'neus_facto_mlx.py'))
mx, nn, optim, np = g['mx'], g['nn'], g['optim'], g['np']
def target(p): return mx.sqrt((p * p * mx.array([1.0, 4.0, 1.0])).sum(-1)) - 0.4          # an ellipsoid-like field
for mode in ('separate', 'eik-only-separate'):
    field = g['Field'](); mx.eval(field.parameters())
    opt = optim.Adam(learning_rate=1e-4 if mode == 'nested-lowlr' else 1e-3, eps=1e-15)
    def loss(p):
        if mode == 'plain':
            return mx.abs(field.distance(p) - target(p)).mean()
        d = field.distance(p); grad = mx.grad(lambda q: field.distance(q).sum())(p)
        return mx.abs(d - target(p)).mean() + 0.1 * ((mx.sqrt((grad * grad).sum(-1) + 1e-12) - 1.0) ** 2).mean()
    vg = nn.value_and_grad(field, loss); row = []
    for n in range(301):
        p = mx.random.uniform(-1, 1, (8192, 3)); v, gr = vg(p); opt.update(field, gr); mx.eval(field.parameters(), opt.state, v)
        if n % 50 == 0: row.append(round(float(v), 4))
    print(mode, row, flush=True)
# analytic against numerical gradient at the start
field = g['Field'](); p = mx.random.uniform(-0.8, 0.8, (5, 3)); d, _, grad = field.with_gradient(p); h = 1e-3
num = mx.stack([(field.distance(p + mx.array([h if a == k else 0.0 for a in range(3)])) - field.distance(p - mx.array([h if a == k else 0.0 for a in range(3)]))) / (2 * h) for k in range(3)], -1)
print('gradient error', float(mx.abs(grad - num).max()))
