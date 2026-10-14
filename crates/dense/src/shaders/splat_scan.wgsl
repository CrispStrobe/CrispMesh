// Exclusive prefix sum of `count` unsigned values in place, by one workgroup: each invocation sums a contiguous
// chunk, the 64 chunk sums are scanned, then each chunk is rewritten. `total` receives the sum of all values.

struct Params { count: u32, pad0: u32, pad1: u32, pad2: u32 }

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> values: array<u32>;
@group(0) @binding(2) var<storage, read_write> total: array<u32>;

var<workgroup> sums: array<u32, 64>;

@compute @workgroup_size(64)
fn main(@builtin(local_invocation_index) t: u32) {
    let chunk = (params.count + 63u) / 64u;
    let start = min(t * chunk, params.count);
    let end = min(start + chunk, params.count);
    var sum = 0u;
    for (var i = start; i < end; i++) { sum += values[i]; }
    sums[t] = sum;
    workgroupBarrier();
    if (t == 0u) {
        var running = 0u;
        for (var k = 0u; k < 64u; k++) {
            let s = sums[k];
            sums[k] = running;
            running += s;
        }
        total[0] = running;
    }
    workgroupBarrier();
    var running = sums[t];
    for (var i = start; i < end; i++) {
        let v = values[i];
        values[i] = running;
        running += v;
    }
}
