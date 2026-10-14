"""Times the parts of one training step of neus_facto_mlx.py."""
import os, sys, time, runpy
os.environ['STEPS'] = '0'; os.environ['MESH'] = '8'
g = runpy.run_path(os.path.join(os.path.dirname(__file__), 'neus_facto_mlx.py'))
mx = g['mx']; field = g['field']
def timed(name, fn, n=3):
    fn(); t = time.time()
    for _ in range(n): fn()
    print(f'{name}: {(time.time() - t) / n * 1000:.0f} ms', flush=True)
o, d, target, mask = (mx.array(a) for a in g['rays'](2048))
p = mx.random.uniform(-0.5, 0.5, (2048 * 48, 3))
timed('hash features 98k', lambda: mx.eval(g['hash_features'](field.table, (p + 2) / 4)))
timed('distance 98k', lambda: mx.eval(field.distance(p)))
timed('distance+gradient 98k', lambda: mx.eval(field.with_gradient(p)))
t0 = time.time(); cache = g['cache_distances'](field); mx.eval(cache); print('cache', time.time() - t0)
timed('sampler', lambda: mx.eval(g['sample'](cache, o, d, field.sharpness())))
edges = g['sample'](cache, o, d, field.sharpness()); mx.eval(edges)
timed('render forward', lambda: mx.eval(g['render'](field, o, d, edges, 0.0)))
vg = g['value_and_grad']
timed('loss+grads', lambda: mx.eval(vg(o, d, edges, target, mask, 0.0)))
opt = g['optimizer']
def full():
    (l, _), gr = vg(o, d, edges, target, mask, 0.0); opt.update(field, gr); mx.eval(field.parameters(), opt.state, l)
timed('full step', full)
print('peak', mx.get_peak_memory() / 2**30)
from functools import partial
state = [field.state, opt.state]
@partial(mx.compile, inputs=state, outputs=state)
def cstep(o, d, edges, target, mask, anneal):
    (l, parts), gr = vg(o, d, edges, target, mask, anneal); opt.update(field, gr); return l, parts
def cfull():
    l, parts = cstep(o, d, edges, target, mask, mx.array(0.0)); mx.eval(state, l)
timed('compiled full step', cfull, 5)
print('peak', mx.get_peak_memory() / 2**30)
