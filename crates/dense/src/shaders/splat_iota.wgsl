// values[i] = i.

struct Params { count: u32, pad0: u32, pad1: u32, pad2: u32 }

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> values: array<u32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x + id.y * ROW;
    if (i >= params.count) { return; }
    values[i] = i;
}
