// SSIM per pixel and channel from the blurred maps (mu_a, mu_b, E[a^2], E[b^2], E[ab]), and the three maps whose
// blur gives its gradient with respect to b: dS/d mu_b, dS/dE[b^2], dS/dE[ab], each times `scale` (the loss's
// derivative with respect to S). out: grads[p * 9 + c * 3 + k], ssim[p] (sum over the channels).

struct Params { width: u32, height: u32, pad0: u32, pad1: u32, scale: f32, pad2: f32, pad3: f32, pad4: f32 }

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> stats: array<f32>;
@group(0) @binding(2) var<storage, read_write> grads: array<f32>;
@group(0) @binding(3) var<storage, read_write> ssim: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = id.x + id.y * ROW;
    if (p >= params.width * params.height) { return; }
    let c1 = 0.01 * 0.01;
    let c2 = 0.03 * 0.03;
    var total = 0.0;
    for (var c = 0u; c < 3u; c++) {
        let o = p * 15u + c * 5u;
        let mu_a = stats[o];
        let m = stats[o + 1u];
        let var_a = stats[o + 2u] - mu_a * mu_a;
        let e = stats[o + 3u];
        let x = stats[o + 4u];
        let A1 = 2.0 * mu_a * m + c1;
        let A2 = 2.0 * (x - mu_a * m) + c2;
        let B1 = mu_a * mu_a + m * m + c1;
        let B2 = var_a + (e - m * m) + c2;
        let s = (A1 * A2) / (B1 * B2);
        total += s;
        let g = p * 9u + c * 3u;
        grads[g] = params.scale * s * (2.0 * mu_a / A1 - 2.0 * mu_a / A2 - 2.0 * m / B1 + 2.0 * m / B2);
        grads[g + 1u] = params.scale * (-s / B2);
        grads[g + 2u] = params.scale * (2.0 * s / A2);
    }
    ssim[p] = total;
}
