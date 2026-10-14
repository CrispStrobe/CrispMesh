// Start and end of each tile's run in the tile-sorted pairs (ranges cleared to zero before).

struct Params { count: u32, pad0: u32, pad1: u32, pad2: u32 }

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> keys: array<u32>;
@group(0) @binding(2) var<storage, read_write> ranges: array<u32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let j = id.x + id.y * ROW;
    if (j >= params.count) { return; }
    let tile = keys[j];
    if (j == 0u || keys[j - 1u] != tile) { ranges[tile * 2u] = j; }
    if (j + 1u == params.count || keys[j + 1u] != tile) { ranges[tile * 2u + 1u] = j + 1u; }
}
