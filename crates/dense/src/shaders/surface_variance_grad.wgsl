// Gradient of the variance parameter (sharpness = exp(10 variance), clipped to [1e-6, 1e6]) from the per-ray
// sharpness gradients of surface_render.

struct Params { rays: u32, variance: u32, pad0: u32, pad1: u32 }

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> losses: array<f32>;
@group(0) @binding(2) var<storage, read> parameters: array<f32>;
@group(0) @binding(3) var<storage, read_write> grad: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x + id.y * ROW != 0u) { return; }
    var total = 0.0;
    for (var r = 0u; r < params.rays; r++) {
        total += losses[r * 4u + 3u];
    }
    let raw = exp(parameters[params.variance] * 10.0);
    if (raw >= 1e-6 && raw <= 1e6) {
        grad[params.variance] += total * 10.0 * raw;
    }
}
