"""Which outer gradient is wrong: through the extra output, or through the inner gradient?"""
import mlx.core as mx, numpy as np
mx.random.seed(0)
w = mx.random.normal((4, 3)); x = mx.random.normal((5, 3))
net = lambda w, q: mx.tanh(q @ w.T).sum(-1)
def inner(w):
    def total(q):
        v = net(w, q); return v.sum(), v
    return mx.value_and_grad(total)(x)
for name, aux, ref in [
    ('extra output only', lambda w: (inner(w)[0][1] ** 2).sum(), lambda w: (net(w, x) ** 2).sum()),
    ('inner gradient only', lambda w: (inner(w)[1] ** 2).sum(), lambda w: (mx.grad(lambda q: net(w, q).sum())(x) ** 2).sum()),
    ('first output only', lambda w: inner(w)[0][0] ** 2, lambda w: net(w, x).sum() ** 2)]:
    print('%-20s max difference %.2e' % (name, float(mx.abs(mx.grad(aux)(w) - mx.grad(ref)(w)).max())))
g = mx.grad(lambda w: (inner(w)[0][1] ** 2).sum())(w); r = mx.grad(lambda w: (net(w, x) ** 2).sum())(w)
print('through extra output: |grad| %.3f, expected |grad| %.3f, ratio to expected %.3f' % (float(mx.abs(g).max()), float(mx.abs(r).max()), float((g * r).sum() / (r * r).sum())))
