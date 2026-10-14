// Adam over the splat rows (60 floats each) with one learning rate per parameter group, as torch.optim.Adam (bias
// correction; `correction1` = 1 - beta1^t, `correction2` = 1 - beta2^t): mean, quaternion, scales, opacity,
// SH constant term, the other SH coefficients; the unused column stays.

struct Params {
    count: u32, pad0: u32, correction1: f32, correction2: f32,
    rates0: vec4<f32>,   // mean, quaternion, scales, opacity
    rates1: vec4<f32>,   // SH constant term, other SH, unused, unused
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> splats: array<f32>;
@group(0) @binding(2) var<storage, read> grads: array<f32>;
@group(0) @binding(3) var<storage, read_write> first: array<f32>;
@group(0) @binding(4) var<storage, read_write> second: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let e = id.x + id.y * ROW;
    if (e >= params.count * 60u) { return; }
    let column = e % 60u;
    var rate = 0.0;
    if (column < 3u) { rate = params.rates0.x; }
    else if (column < 7u) { rate = params.rates0.y; }
    else if (column < 10u) { rate = params.rates0.z; }
    else if (column < 11u) { rate = params.rates0.w; }
    else if (column < 14u) { rate = params.rates1.x; }
    else if (column < 59u) { rate = params.rates1.y; }
    if (rate == 0.0) { return; }
    let g = grads[e];
    let m = 0.9 * first[e] + 0.1 * g;
    let v = 0.999 * second[e] + 0.001 * g * g;
    first[e] = m;
    second[e] = v;
    splats[e] -= (rate / params.correction1) * m / (sqrt(v) / sqrt(params.correction2) + 1e-15);
}
