// Bias gradient: the sum of the output gradient over the value rows (every `stride`-th row), added to `grad`.
// One workgroup per output column: 64 partial sums, then a tree in workgroup memory.

struct Params { rows: u32, width: u32, stride: u32, dst: u32 }

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> dz: array<f32>;
@group(0) @binding(2) var<storage, read_write> grad: array<f32>;

var<workgroup> partial: array<f32, 64>;

@compute @workgroup_size(64)
fn main(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let o = group.x + group.y * (ROW / 64u);
    if (o >= params.width) { return; }
    var total = 0.0;
    for (var r = t * params.stride; r < params.rows; r += 64u * params.stride) {
        total += dz[r * params.width + o];
    }
    partial[t] = total;
    workgroupBarrier();
    for (var half = 32u; half > 0u; half >>= 1u) {
        if (t < half) {
            partial[t] += partial[t + half];
        }
        workgroupBarrier();
    }
    if (t == 0u) {
        grad[params.dst + o] += partial[0];
    }
}
