"""Does mx.vjp give the value and the input gradient in one pass, both differentiable by an outer transform?"""
import mlx.core as mx
mx.random.seed(0)
w = mx.random.normal((4, 3)); x = mx.random.normal((5, 3))
net = lambda w, q: mx.tanh(q @ w.T).sum(-1)
def one_pass(w):
    (v,), (g,) = mx.vjp(lambda q: [net(w, q)], [x], [mx.ones((5,))]); return (v ** 2).sum() + (g ** 2).sum()
def separate(w):
    return (net(w, x) ** 2).sum() + (mx.grad(lambda q: net(w, q).sum())(x) ** 2).sum()
print('vjp one pass vs separate: max difference %.2e' % float(mx.abs(mx.grad(one_pass)(w) - mx.grad(separate)(w)).max()))
