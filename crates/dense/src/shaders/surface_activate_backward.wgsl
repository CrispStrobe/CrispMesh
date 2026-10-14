// Gradient through surface_activate, in place: `g` holds the gradient of h and receives the gradient of z.
// Dual rows: the value's gradient also collects the tangents' (act''(z) times tangent times its gradient).

struct Params { count: u32, width: u32, dual: u32, kind: u32 }

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> z: array<f32>;
@group(0) @binding(2) var<storage, read_write> g: array<f32>;

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
        let slope = select(select(0.0, 1.0, v > 0.0), sigmoid(v * 100.0), params.kind == 0u);
        g[e] = g[e] * slope;
        return;
    }
    let base = s * 4u * params.width + c;
    let v = z[base];
    var slope = select(0.0, 1.0, v > 0.0);
    var curvature = 0.0;
    if (params.kind == 0u) {
        slope = sigmoid(v * 100.0);
        curvature = 100.0 * slope * (1.0 - slope);
    }
    var value = g[base] * slope;
    for (var k = 1u; k < 4u; k++) {
        let index = base + k * params.width;
        value += curvature * z[index] * g[index];
        g[index] = g[index] * slope;
    }
    g[base] = value;
}
