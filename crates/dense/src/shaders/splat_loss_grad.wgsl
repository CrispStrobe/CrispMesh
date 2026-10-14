// The loss's gradient with respect to the rendered images, and each pixel's loss terms. Loss (means over pixels, and
// colour channels where they apply): l1 |b - a| + ssim (1 - SSIM) (its gradient blurred in `ssim_grads`) + mask
// |alpha - mask| + normal (1 - n . s alpha) with n the rendered normal in the world, s the median depth's normal and
// alpha held fixed + distortion. Out: dpixels[p * 12]: colour (3), alpha, camera normal (3), distortion (the median's
// gradient follows from splat_median_grad); dsurface[p * 4]: the gradient of s; terms[p * 5]: |b - a| summed over
// the channels, SSIM summed over them, |alpha - mask|, 1 - n . s alpha, distortion.

struct Params {
    width: u32, height: u32, pad0: u32, pad1: u32,
    l1: f32, mask: f32, normal: f32, distortion: f32,
    background: vec4<f32>,
    rotation0: vec4<f32>, rotation1: vec4<f32>, rotation2: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> pixels: array<f32>;
@group(0) @binding(2) var<storage, read> maps: array<f32>;
@group(0) @binding(3) var<storage, read> ssim_grads: array<f32>;
@group(0) @binding(4) var<storage, read> photo: array<u32>;
@group(0) @binding(5) var<storage, read> surface: array<f32>;
@group(0) @binding(6) var<storage, read_write> dpixels: array<f32>;
@group(0) @binding(7) var<storage, read_write> dsurface: array<f32>;
@group(0) @binding(8) var<storage, read_write> terms: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let count = params.width * params.height;
    let p = id.x + id.y * ROW;
    if (p >= count) { return; }
    let inv = 1.0 / f32(count);
    let mask = select(0.0, 1.0, unpack4x8unorm(photo[p]).w > 0.5);
    let alpha = pixels[p * 16u + 3u];
    var l1 = 0.0;
    var dalpha = 0.0;
    for (var c = 0u; c < 3u; c++) {
        let a = maps[p * 15u + c * 5u];
        let b = maps[p * 15u + c * 5u + 1u];
        let g = p * 9u + c * 3u;
        let db = params.l1 * sign(b - a) * inv / 3.0 + ssim_grads[g] + 2.0 * b * ssim_grads[g + 1u] + a * ssim_grads[g + 2u];
        dpixels[p * 12u + c] = db;
        dalpha -= db * params.background[c];
        l1 += abs(b - a);
    }
    dalpha += params.mask * sign(alpha - mask) * inv;
    dpixels[p * 12u + 3u] = dalpha;
    // Rendered normal in the world: R^T n.
    let nc = vec3<f32>(pixels[p * 16u + 4u], pixels[p * 16u + 5u], pixels[p * 16u + 6u]);
    let nw = params.rotation0.xyz * nc.x + params.rotation1.xyz * nc.y + params.rotation2.xyz * nc.z;
    let s = vec3<f32>(surface[p * 8u], surface[p * 8u + 1u], surface[p * 8u + 2u]);
    let agreement = dot(nw, s) * alpha;
    let dnw = -params.normal * inv * s * alpha;
    // The camera normal's gradient is R times the world normal's.
    dpixels[p * 12u + 4u] = dot(params.rotation0.xyz, dnw);
    dpixels[p * 12u + 5u] = dot(params.rotation1.xyz, dnw);
    dpixels[p * 12u + 6u] = dot(params.rotation2.xyz, dnw);
    dpixels[p * 12u + 7u] = params.distortion * inv;
    dpixels[p * 12u + 8u] = 0.0;
    let ds = -params.normal * inv * nw * alpha;
    dsurface[p * 4u] = ds.x;
    dsurface[p * 4u + 1u] = ds.y;
    dsurface[p * 4u + 2u] = ds.z;
    dsurface[p * 4u + 3u] = 0.0;
    terms[p * 5u] = l1;
    terms[p * 5u + 2u] = abs(alpha - mask);
    terms[p * 5u + 3u] = 1.0 - agreement;
    terms[p * 5u + 4u] = pixels[p * 16u + 8u];
}
