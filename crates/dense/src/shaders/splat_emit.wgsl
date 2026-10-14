// One (tile, splat) pair per covered tile, in depth order: splat order[i] writes its tiles (row by row) from
// offsets[i] on. A stable sort by tile then gives each tile its splats front to back.

struct Params { count: u32, tiles_x: u32, pad0: u32, pad1: u32 }

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> order: array<u32>;
@group(0) @binding(2) var<storage, read> projected: array<f32>;
@group(0) @binding(3) var<storage, read> offsets: array<u32>;
@group(0) @binding(4) var<storage, read_write> keys: array<u32>;
@group(0) @binding(5) var<storage, read_write> values: array<u32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x + id.y * ROW;
    if (i >= params.count) { return; }
    let g = order[i];
    let base = g * 24u;
    let count = bitcast<u32>(projected[base + 23u]);
    if (count == 0u) { return; }
    let lo = bitcast<u32>(projected[base + 21u]);
    let hi = bitcast<u32>(projected[base + 22u]);
    var slot = offsets[i];
    for (var y = lo >> 16u; y < (hi >> 16u); y++) {
        for (var x = lo & 0xffffu; x < (hi & 0xffffu); x++) {
            keys[slot] = y * params.tiles_x + x;
            values[slot] = g;
            slot += 1u;
        }
    }
}
