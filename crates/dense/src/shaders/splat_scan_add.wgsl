// Exclusive prefix sum, last phase: each value gets the scanned total of the blocks before its own.

struct Params { count: u32, blocks: u32, pad0: u32, pad1: u32 }

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> values: array<u32>;
@group(0) @binding(2) var<storage, read> sums: array<u32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x + id.y * ROW;
    if (i >= params.count) { return; }
    values[i] += sums[i / 1024u];
}
