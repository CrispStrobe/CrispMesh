"""Reference values for the native 2D Gaussian splatting (2DGS): a dense PyTorch renderer that follows gsplat
(Apache-2.0, commit 937e299: _torch_impl_2dgs.py, RasterizeToPixels2DGSFwd.cu, rendering.rasterization_2dgs,
utils.depth_to_normal) and the loss of our trainer after gsplat's examples/simple_trainer_2dgs.py, written anew.
Every pixel is composited over all splats in depth order, restricted to the splats whose tile rectangle covers the
pixel's tile, as gsplat's tiled renderer does. Small scenes only; gradients by autograd.

usage: splat_reference.py OUT.npz [SEED]"""
import sys
import numpy as np
import torch
import torch.nn.functional as F

torch.set_default_dtype(torch.float32)
TILE = 16
ALPHA_THRESHOLD = 1.0 / 255.0
SH_C0 = 0.2820947917738781


def rotation(quats):
    q = F.normalize(quats, dim=-1)
    w, x, y, z = q.unbind(-1)
    return torch.stack([1 - 2 * (y * y + z * z), 2 * (x * y - w * z), 2 * (x * z + w * y),
                        2 * (x * y + w * z), 1 - 2 * (x * x + z * z), 2 * (y * z - w * x),
                        2 * (x * z - w * y), 2 * (y * z + w * x), 1 - 2 * (x * x + y * y)], -1).reshape(q.shape[:-1] + (3, 3))


def sh_bases(degree, dirs):
    """Real spherical harmonics up to `degree` (gsplat's _eval_sh_bases_fast) at unit directions."""
    x, y, z = dirs.unbind(-1)
    out = [torch.full_like(x, SH_C0)]
    if degree >= 1:
        a = -0.48860251190292
        out += [a * y, -a * z, a * x]
    if degree >= 2:
        z2 = z * z; b = -1.092548430592079 * z; a = 0.5462742152960395; c1 = x * x - y * y; s1 = 2 * x * y
        out += [a * s1, b * y, 0.9461746957575601 * z2 - 0.3153915652525201, b * x, a * c1]
    if degree >= 3:
        z2 = z * z; c1 = x * x - y * y; s1 = 2 * x * y
        tc = -2.285228997322329 * z2 + 0.4570457994644658; tb = 1.445305721320277 * z; ta = -0.5900435899266435
        c2 = x * c1 - y * s1; s2 = x * s1 + y * c1
        out += [ta * s2, tb * s1, tc * y, z * (1.865881662950577 * z2 - 1.119528997770346), tc * x, tb * c1, ta * c2]
    return torch.stack(out, -1)


def project(means, quats, scales, viewmat, K, width, height, near=0.2, far=1e10):
    """gsplat's 2DGS projection for one camera: radii, means2d, depths, ray transforms (rows), normals (camera)."""
    R, t = viewmat[:3, :3], viewmat[:3, 3]
    means_c = means @ R.T + t
    # gsplat's CUDA projection (which training uses) takes the third scale as 1, so the normal is a unit vector; its
    # PyTorch reference keeps the scale there.
    RS = rotation(quats) * torch.cat([scales[:, :2], torch.ones_like(scales[:, 2:])], -1)[:, None, :]
    RS_c = R[None] @ RS
    normals = RS_c[..., 2]
    cos = -(normals * means_c).sum(-1, keepdim=True)
    normals = normals * torch.where(cos > 0, 1.0, -1.0)
    T_c = torch.cat([RS_c[..., :2], means_c[..., None]], -1)
    M = (K[None] @ T_c).transpose(-1, -2)                     # paper notation M = (WH)^T
    test = torch.tensor([1.0, 1.0, -1.0])
    d = (M[..., 2] * M[..., 2] * test).sum(-1, keepdim=True)
    valid = d.abs() > 0
    f = torch.where(valid, test / d, torch.zeros_like(test))[..., None]
    means2d = (M[..., :2] * M[..., 2:3] * f).sum(-2)
    extents = torch.sqrt((means2d ** 2 - (M[..., :2] * M[..., :2] * f).sum(-2)).clamp_min(1e-4))
    depths = means_c[:, 2]
    radius = torch.ceil(3.33 * extents).detach()
    valid = valid[:, 0] & (depths > near) & (depths < far)
    radius = torch.where(valid[:, None], radius, torch.zeros_like(radius))
    inside = (means2d[:, 0] + radius[:, 0] > 0) & (means2d[:, 0] - radius[:, 0] < width) & (means2d[:, 1] + radius[:, 1] > 0) & (means2d[:, 1] - radius[:, 1] < height)
    radius = torch.where(inside[:, None], radius, torch.zeros_like(radius))
    return radius.int(), means2d, depths, M.transpose(-1, -2), normals


def render(params, viewmat, K, width, height, degree):
    """Colours (H, W, 3), alpha, normals (world), expected depth, median depth, distortion, and the projection."""
    means, quats, scales, opacities = params["means"], params["quats"], torch.exp(params["scales"]), torch.sigmoid(params["opacities"])
    radii, means2d, depths, M, normals = project(means, quats, scales, viewmat, K, width, height)
    # Intermediate values whose gradients the fixture keeps (to test the backward kernels one by one).
    for v in (opacities, means2d, depths, M, normals):
        if v.requires_grad:
            v.retain_grad()
    camera = torch.linalg.inv(viewmat)[:3, 3]
    dirs = F.normalize(means - camera, dim=-1)
    coeffs = torch.cat([params["sh0"], params["shN"]], 1)
    colours = torch.clamp_min((sh_bases(degree, dirs)[..., None] * coeffs[:, :(degree + 1) ** 2]).sum(1) + 0.5, 0.0)
    if colours.requires_grad:
        colours.retain_grad()
    tiles_w, tiles_h = (width + TILE - 1) // TILE, (height + TILE - 1) // TILE
    tile_r = radii.float() / TILE
    tile_mean = means2d.detach() / TILE
    lo = torch.stack([torch.floor(tile_mean[:, 0] - tile_r[:, 0]).clamp(0, tiles_w), torch.floor(tile_mean[:, 1] - tile_r[:, 1]).clamp(0, tiles_h)], -1)
    hi = torch.stack([torch.ceil(tile_mean[:, 0] + tile_r[:, 0]).clamp(0, tiles_w), torch.ceil(tile_mean[:, 1] + tile_r[:, 1]).clamp(0, tiles_h)], -1)
    shown = (radii > 0).all(-1)
    ys, xs = torch.meshgrid(torch.arange(height), torch.arange(width), indexing="ij")
    px, py = xs.reshape(-1).float() + 0.5, ys.reshape(-1).float() + 0.5
    tx, ty = xs.reshape(-1) // TILE, ys.reshape(-1) // TILE
    P = px.shape[0]
    T = torch.ones(P); done = torch.zeros(P, dtype=torch.bool)
    out = torch.zeros(P, 3); depth_sum = torch.zeros(P); normal_out = torch.zeros(P, 3)
    distort = torch.zeros(P); accum = torch.zeros(P); median = torch.zeros(P)
    for g in torch.argsort(depths.detach(), stable=True).tolist():
        if not shown[g]:
            continue
        covered = (tx >= lo[g, 0]) & (tx < hi[g, 0]) & (ty >= lo[g, 1]) & (ty < hi[g, 1])
        if not covered.any():
            continue
        h_u = px[:, None] * M[g, 2][None] - M[g, 0][None]
        h_v = py[:, None] * M[g, 2][None] - M[g, 1][None]
        cross = torch.cross(h_u, h_v, dim=-1)
        nonzero = cross[:, 2] != 0
        z = torch.where(nonzero, cross[:, 2], torch.ones_like(cross[:, 2]))
        s = cross[:, :2] / z[:, None]
        weight_3d = (s * s).sum(-1)
        delta = means2d[g][None] - torch.stack([px, py], -1)
        weight_2d = 2.0 * (delta * delta).sum(-1)
        sigma = 0.5 * torch.minimum(weight_3d, weight_2d)
        alpha = torch.clamp_max(opacities[g] * torch.exp(-sigma), 0.999)
        use = covered & ~done & nonzero & (sigma >= 0) & (alpha >= ALPHA_THRESHOLD)
        next_T = T * (1 - alpha)
        stop = use & (next_T <= 1e-4)
        done = done | stop
        use = use & ~stop
        vis = torch.where(use, alpha * T, torch.zeros_like(T))
        out = out + vis[:, None] * colours[g][None]
        depth_sum = depth_sum + vis * depths[g]
        normal_out = normal_out + vis[:, None] * normals[g][None]
        distort = distort + 2.0 * (vis * depths[g] * (1 - T) - vis * accum)
        accum = accum + vis * depths[g]
        median = torch.where(use & (T > 0.5), depths[g].expand(P), median)
        T = torch.where(use, next_T, T)
    alpha = 1 - T
    world_normals = normal_out @ viewmat[:3, :3]                 # R^T n per row
    expected = depth_sum / alpha.clamp(min=1e-10)
    shape = (height, width)
    for v in (out, alpha, world_normals, median, distort):
        if v.requires_grad:
            v.retain_grad()
    return {"colours": out.reshape(shape + (3,)), "alpha": alpha.reshape(shape), "normals": world_normals.reshape(shape + (3,)),
            "expected_depth": expected.reshape(shape), "median": median.reshape(shape), "distort": distort.reshape(shape),
            "radii": radii, "means2d": means2d, "depths": depths, "ray_transforms": M, "camera_normals": normals, "rgb": colours,
            "opacity": opacities, "flat": {"colours": out, "alpha": alpha, "normals": world_normals, "median": median, "distort": distort}}


def depth_to_normal(depth, viewmat, K):
    """gsplat's depth_to_normal (z depth, pixel centres at + 0.5), world normals, zero on the border."""
    H, W = depth.shape
    x, y = torch.meshgrid(torch.arange(W), torch.arange(H), indexing="xy")
    dirs = torch.stack([(x - K[0, 2] + 0.5) / K[0, 0], (y - K[1, 2] + 0.5) / K[1, 1], torch.ones(H, W)], -1)
    c2w = torch.linalg.inv(viewmat)
    points = c2w[:3, 3] + depth[..., None] * (dirs @ c2w[:3, :3].T)
    dx = points[2:, 1:-1] - points[:-2, 1:-1]
    dy = points[1:-1, 2:] - points[1:-1, :-2]
    normals = F.normalize(torch.cross(dx, dy, dim=-1), dim=-1)
    return F.pad(normals, (0, 0, 1, 1, 1, 1))


def ssim(a, b):
    """Mean SSIM of two [1, 3, H, W] images, 11 x 11 Gaussian window, sigma 1.5 (our trainer's)."""
    g = torch.exp(-((torch.arange(11) - 5.0) ** 2) / (2 * 1.5 ** 2)); g = g / g.sum()
    window = (g[:, None] * g[None, :])[None, None].repeat(3, 1, 1, 1)
    mu_a, mu_b = F.conv2d(a, window, padding=5, groups=3), F.conv2d(b, window, padding=5, groups=3)
    var_a = F.conv2d(a * a, window, padding=5, groups=3) - mu_a ** 2; var_b = F.conv2d(b * b, window, padding=5, groups=3) - mu_b ** 2
    cov = F.conv2d(a * b, window, padding=5, groups=3) - mu_a * mu_b
    c1, c2 = 0.01 ** 2, 0.03 ** 2
    return (((2 * mu_a * mu_b + c1) * (2 * cov + c2)) / ((mu_a ** 2 + mu_b ** 2 + c1) * (var_a + var_b + c2))).mean()


def loss_terms(result, pixels, mask, background, viewmat, K):
    """Our trainer's loss with every term on: 0.8 L1 + 0.2 (1 - SSIM), 0.1 mask L1, 0.05 normal agreement, 0.01 distortion."""
    alpha = result["alpha"][..., None]
    target = pixels * mask[..., None] + background * (1 - mask[..., None])
    rendered = result["colours"] + background * (1 - alpha)
    l1 = (rendered - target).abs().mean()
    s = ssim(target.permute(2, 0, 1)[None], rendered.permute(2, 0, 1)[None])
    mask_l1 = (alpha[..., 0] - mask).abs().mean()
    surface = depth_to_normal(result["median"], viewmat, K)
    agreement = (result["normals"] * (surface * alpha.detach())).sum(-1)
    normal = (1 - agreement).mean()
    distort = result["distort"].mean()
    total = 0.8 * l1 + 0.2 * (1 - s) + 0.1 * mask_l1 + 0.05 * normal + 0.01 * distort
    return total, {"l1": l1, "ssim": s, "mask_l1": mask_l1, "normal": normal, "distortion": distort, "surface_normals": surface}


def scene(seed):
    """A small random scene: splats in a ball, one camera looking at it, a smooth target with a mask."""
    r = np.random.default_rng(seed)
    N, degree, W, H = 160, 3, 72, 56
    means = r.normal(0, 0.35, (N, 3))
    params = {"means": means, "quats": r.normal(size=(N, 4)), "scales": np.log(r.uniform(0.04, 0.15, (N, 3))),
              "opacities": r.normal(0.5, 1.0, N), "sh0": r.normal(0, 0.5, (N, 1, 3)), "shN": r.normal(0, 0.15, (N, (degree + 1) ** 2 - 1, 3))}
    eye = np.array([0.3, -0.4, -2.6]); forward = -eye / np.linalg.norm(eye); right = np.cross([0, 1, 0], forward); right /= np.linalg.norm(right)
    down = np.cross(forward, right)
    R = np.stack([right, down, forward]); viewmat = np.eye(4); viewmat[:3, :3] = R; viewmat[:3, 3] = -R @ eye
    K = np.array([[70.0, 0, W / 2 + 0.3], [0, 70.0, H / 2 - 0.4], [0, 0, 1]])
    yy, xx = np.mgrid[0:H, 0:W]
    pixels = np.round(np.stack([0.5 + 0.4 * np.sin(xx / 7.0 + c) * np.cos(yy / 9.0 - c) for c in range(3)], -1) * 255) / 255   # 8-bit, as photos are
    mask = (((xx - W / 2) / (W / 3)) ** 2 + ((yy - H / 2) / (H / 3)) ** 2 < 1).astype(np.float64)
    return params, viewmat, K, W, H, degree, pixels, mask, np.array([0.2, 0.6, 0.9])


def main():
    out = sys.argv[1]; seed = int(sys.argv[2]) if len(sys.argv) > 2 else 0
    raw, viewmat, K, W, H, degree, pixels, mask, background = scene(seed)
    params = {k: torch.tensor(v, dtype=torch.float32, requires_grad=True) for k, v in raw.items()}
    viewmat, K = torch.tensor(viewmat, dtype=torch.float32), torch.tensor(K, dtype=torch.float32)
    pixels, mask, background = (torch.tensor(a, dtype=torch.float32) for a in (pixels, mask, background))
    result = render(params, viewmat, K, W, H, degree)
    total, terms = loss_terms(result, pixels, mask, background, viewmat, K)
    total.backward()
    arrays = {f"param/{k}": v.detach() for k, v in params.items()}
    arrays.update({f"grad/{k}": v.grad for k, v in params.items()})
    flat = result.pop("flat")
    # Gradients of the loss with respect to the rendered images (per pixel) and to the projected splats.
    arrays.update({f"dimage/{k}": v.grad for k, v in flat.items() if v.grad is not None})
    arrays.update({f"dsplat/{k}": result[k].grad for k in ("means2d", "depths", "ray_transforms", "camera_normals", "rgb", "opacity")})
    arrays.update({k: v.detach() for k, v in result.items()})
    arrays.update({k: v.detach() for k, v in terms.items()})
    arrays.update({"viewmat": viewmat, "K": K, "pixels": pixels, "mask": mask, "background": background, "loss": total.detach(),
                   "size": torch.tensor([W, H, degree])})
    np.savez(out, **{k: np.asarray(v.numpy()) for k, v in arrays.items()})
    shown = int((result["radii"] > 0).all(-1).sum())
    print(f"wrote {out}: loss {float(total):.5f} (l1 {float(terms['l1']):.4f} ssim {float(terms['ssim']):.4f} mask {float(terms['mask_l1']):.4f} "
          f"normal {float(terms['normal']):.4f} distortion {float(terms['distortion']):.5f}), {shown} splats shown, mean alpha {float(result['alpha'].mean()):.3f}")
    for k, v in params.items():
        print(f"  grad/{k}: |g| {float(v.grad.norm()):.4e}")


if __name__ == "__main__":
    main()
