// Radix sort, pass 3 of 3: stable scatter. Each invocation owns 16 consecutive keys of the block; its first slot per
// digit is the block's offset (scanned histogram) plus the counts of that digit in the invocations before it.

struct Params { count: u32, shift: u32, blocks: u32, pad: u32 }

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> keys: array<u32>;
@group(0) @binding(2) var<storage, read> values: array<u32>;
@group(0) @binding(3) var<storage, read> offsets: array<u32>;
@group(0) @binding(4) var<storage, read_write> keys_out: array<u32>;
@group(0) @binding(5) var<storage, read_write> values_out: array<u32>;

// counts[digit * 64 + invocation], then turned into exclusive prefixes over the invocations.
var<workgroup> counts: array<u32, 1024>;

@compute @workgroup_size(64)
fn main(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let block = group.x + group.y * (ROW / 64u);
    if (block >= params.blocks) { return; }
    for (var d = 0u; d < 16u; d++) { counts[d * 64u + t] = 0u; }
    let start = block * 1024u + t * 16u;
    for (var i = 0u; i < 16u; i++) {
        let index = start + i;
        if (index < params.count) {
            let d = (keys[index] >> params.shift) & 15u;
            counts[d * 64u + t] += 1u;
        }
    }
    workgroupBarrier();
    if (t < 16u) {
        var running = offsets[t * params.blocks + block];
        for (var k = 0u; k < 64u; k++) {
            let c = counts[t * 64u + k];
            counts[t * 64u + k] = running;
            running += c;
        }
    }
    workgroupBarrier();
    for (var i = 0u; i < 16u; i++) {
        let index = start + i;
        if (index < params.count) {
            let key = keys[index];
            let d = (key >> params.shift) & 15u;
            let slot = counts[d * 64u + t];
            counts[d * 64u + t] = slot + 1u;
            keys_out[slot] = key;
            values_out[slot] = values[index];
        }
    }
}
