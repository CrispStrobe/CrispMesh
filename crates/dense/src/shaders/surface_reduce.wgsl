// Adds the per-chunk partial weight gradients of surface_matmul (mode 2) into the gradient buffer.

struct Params { count: u32, chunks: u32, dst: u32, pad: u32 }

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> partial: array<f32>;
@group(0) @binding(2) var<storage, read_write> grad: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let e = id.x + id.y * ROW;
    if (e >= params.count) { return; }
    var total = 0.0;
    for (var c = 0u; c < params.chunks; c++) {
        total += partial[c * params.count + e];
    }
    grad[params.dst + e] += total;
}
