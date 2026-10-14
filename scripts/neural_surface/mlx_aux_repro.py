"""Minimal check: outer gradients through an inner mx.value_and_grad whose function returns extra outputs."""
import mlx.core as mx, numpy as np
mx.random.seed(0)
w = mx.random.normal((4, 3)); x = mx.random.normal((5, 3))

def net(w, q):
    return mx.tanh(q @ w.T).sum(-1)                      # one value per point

def loss_aux(w):
    def total(q):
        v = net(w, q); return v.sum(), v
    (_, v), g = mx.value_and_grad(total)(x)             # value v as an extra output
    return (v ** 2).sum() + (g ** 2).sum()

def loss_separate(w):
    v = net(w, x); g = mx.grad(lambda q: net(w, q).sum())(x)
    return (v ** 2).sum() + (g ** 2).sum()

def loss_numeric(w, h=1e-3):
    out = np.zeros(w.shape); base = np.array(w)
    for i in range(w.shape[0]):
        for j in range(w.shape[1]):
            for sgn in (1, -1):
                b = base.copy(); b[i, j] += sgn * h; out[i, j] += sgn * float(loss_separate(mx.array(b))) / (2 * h)
    return out

print('mlx', mx.__version__)
print('values equal:', float(loss_aux(w)), float(loss_separate(w)))
ga, gs, gn = (np.array(mx.grad(loss_aux)(w)), np.array(mx.grad(loss_separate)(w)), loss_numeric(w))
print('aux form vs numeric:      max error %.2e' % np.abs(ga - gn).max())
print('separate form vs numeric: max error %.2e' % np.abs(gs - gn).max())
