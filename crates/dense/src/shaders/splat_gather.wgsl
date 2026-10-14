// Tile counts in depth order: counts[i] = tiles covered by splat order[i].

struct Params { count: u32, pad0: u32, pad1: u32, pad2: u32 }

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> order: array<u32>;
@group(0) @binding(2) var<storage, read> projected: array<f32>;
@group(0) @binding(3) var<storage, read_write> counts: array<u32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x + id.y * ROW;
    if (i >= params.count) { return; }
    counts[i] = bitcast<u32>(projected[order[i] * 24u + 23u]);
}
