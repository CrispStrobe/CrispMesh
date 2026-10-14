// Radix sort, pass 1 of 3: digit counts (4 bits at `shift`) per block of 1024 keys (64 invocations x 16 consecutive
// keys), written digit-major: histogram[digit * blocks + block].

struct Params { count: u32, shift: u32, blocks: u32, pad: u32 }

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> keys: array<u32>;
@group(0) @binding(2) var<storage, read_write> histogram: array<u32>;

var<workgroup> counts: array<atomic<u32>, 16>;

@compute @workgroup_size(64)
fn main(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let block = group.x + group.y * (ROW / 64u);
    if (block >= params.blocks) { return; }
    if (t < 16u) { atomicStore(&counts[t], 0u); }
    workgroupBarrier();
    let start = block * 1024u + t * 16u;
    for (var i = 0u; i < 16u; i++) {
        let index = start + i;
        if (index < params.count) {
            atomicAdd(&counts[(keys[index] >> params.shift) & 15u], 1u);
        }
    }
    workgroupBarrier();
    if (t < 16u) {
        histogram[t * params.blocks + block] = atomicLoad(&counts[t]);
    }
}
