// h = act(z) per element. kind 0: softplus with beta 100, kind 1: ReLU. With `dual` each point has four rows
// (value, three tangents): the tangents become act'(z) times the tangent.

struct Params { count: u32, width: u32, dual: u32, kind: u32 }

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> z: array<f32>;
@group(0) @binding(2) var<storage, read_write> h: array<f32>;

fn softplus(v: f32) -> f32 {
    let x = v * 100.0;
    return (max(x, 0.0) + log(1.0 + exp(-abs(x)))) * 0.01;
}

fn sigmoid(v: f32) -> f32 {
    return 1.0 / (1.0 + exp(-v));
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let e = id.x + id.y * ROW;
    if (e >= params.count * params.width) { return; }
    let s = e / params.width;
    let c = e % params.width;
    if (params.dual == 0u) {
        let v = z[e];
        h[e] = select(max(v, 0.0), softplus(v), params.kind == 0u);
        return;
    }
    let base = s * 4u * params.width + c;
    let v = z[base];
    var slope = select(0.0, 1.0, v > 0.0);
    if (params.kind == 0u) {
        h[base] = softplus(v);
        slope = sigmoid(v * 100.0);
    } else {
        h[base] = max(v, 0.0);
    }
    for (var k = 1u; k < 4u; k++) {
        h[base + k * params.width] = slope * z[base + k * params.width];
    }
}
