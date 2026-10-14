// Exclusive prefix sum, first phase: within each block of 1024 values (64 invocations x 16 consecutive), in place;
// each block's total goes to sums[block] (scanned next by splat_scan, then added by splat_scan_add).

struct Params { count: u32, blocks: u32, pad0: u32, pad1: u32 }

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> values: array<u32>;
@group(0) @binding(2) var<storage, read_write> sums: array<u32>;

var<workgroup> partial: array<u32, 64>;

@compute @workgroup_size(64)
fn main(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let block = group.x + group.y * (ROW / 64u);
    if (block >= params.blocks) { return; }
    let start = block * 1024u + t * 16u;
    var running = 0u;
    for (var i = 0u; i < 16u; i++) {
        let index = start + i;
        if (index < params.count) {
            let v = values[index];
            values[index] = running;
            running += v;
        }
    }
    partial[t] = running;
    workgroupBarrier();
    // Hillis-Steele scan of the 64 sums (inclusive), then shifted.
    for (var offset = 1u; offset < 64u; offset *= 2u) {
        var add = 0u;
        if (t >= offset) { add = partial[t - offset]; }
        workgroupBarrier();
        partial[t] += add;
        workgroupBarrier();
    }
    let before = select(0u, partial[t - 1u], t > 0u);
    for (var i = 0u; i < 16u; i++) {
        let index = start + i;
        if (index < params.count) { values[index] += before; }
    }
    if (t == 63u) { sums[block] = partial[63]; }
}
