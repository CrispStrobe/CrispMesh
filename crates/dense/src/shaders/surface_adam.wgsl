// Adam: m = b1 m + (1 - b1) g, v = b2 v + (1 - b2) g^2, p -= rate m / (sqrt(v) + eps), with the bias corrections
// folded into `rate` and `correction` by the caller (1 and 1 for none, as MLX's default).

struct Params { count: u32, pad: u32, rate: f32, correction: f32, beta1: f32, beta2: f32, eps: f32, pad1: f32 }

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> parameters: array<f32>;
@group(0) @binding(2) var<storage, read> grad: array<f32>;
@group(0) @binding(3) var<storage, read_write> first: array<f32>;
@group(0) @binding(4) var<storage, read_write> second: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let e = id.x + id.y * ROW;
    if (e >= params.count) { return; }
    let g = grad[e];
    let m = params.beta1 * first[e] + (1.0 - params.beta1) * g;
    let v = params.beta2 * second[e] + (1.0 - params.beta2) * g * g;
    first[e] = m;
    second[e] = v;
    parameters[e] -= params.rate * m / (sqrt(v * params.correction) + params.eps);
}
