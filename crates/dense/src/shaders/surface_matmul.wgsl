// Matrix products of the surface network: 64 x TN outputs per workgroup (TN 64 or 32), 8 x TN / 8 per invocation
// (rows tr + 8 i, columns tc + 8 j), 16 values of the inner dimension per round through workgroup memory. MODE and
// TN are prepended, and
// the accumulation and the stores are written out by `matmul_source` (named variables, no private arrays: indexed
// private arrays are kept in memory, not registers, and made the kernel ten times slower):
// 0, layer forward:    C[m, n] = sum_k A[m, k] B[n, k] + bias[n] on every bias_stride-th row (A: inputs, B: weights)
// 1, input gradient:   C[m, n] = sum_k A[m, k] B[k, n]              (A: output gradients, B: weights)
// 2, weight gradient:  C[chunk, m, n] = sum_{k in chunk} A[k, m] B[k, n]  (A: output gradients, B: inputs)
// Rows of a dual batch (a value and its three tangents) are ordinary rows here: the tangents pass the same linear
// map without the bias (bias_stride 4), and their products add to the weight gradient like the values'.

struct Dims {
    m: u32, n: u32, k: u32, mode: u32,
    a_off: u32, b_off: u32, c_off: u32, bias_off: u32,
    bias_stride: u32, chunk: u32, pad0: u32, pad1: u32,
}

@group(0) @binding(0) var<uniform> dims: Dims;
@group(0) @binding(1) var<storage, read> a: array<f32>;
@group(0) @binding(2) var<storage, read> b: array<f32>;
@group(0) @binding(3) var<storage, read_write> c: array<f32>;
@group(0) @binding(4) var<storage, read> bias: array<f32>;

// Both tiles k-major: tile_a[k * 64 + row], tile_b[k * TN + column].
var<workgroup> tile_a: array<f32, 1024>;
var<workgroup> tile_b: array<f32, 1024>;

fn store(m: u32, n: u32, chunk: u32, value: f32) {
    if (m >= dims.m || n >= dims.n) { return; }
    var v = value;
    if (MODE == 0u && dims.bias_stride > 0u && m % dims.bias_stride == 0u) {
        v += bias[dims.bias_off + n];
    }
    if (MODE == 2u) {
        c[dims.c_off + (chunk * dims.m + m) * dims.n + n] = v;
    } else {
        c[dims.c_off + m * dims.n + n] = v;
    }
}

@compute @workgroup_size(64)
fn main(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let w = group.x + group.y * (ROW / 64u);
    let tiles_m = (dims.m + 63u) / 64u;
    let tiles_n = (dims.n + TN - 1u) / TN;
    let tiles = tiles_m * tiles_n;
    var chunk = 0u;
    var k_start = 0u;
    var k_end = dims.k;
    var chunks = 1u;
    if (MODE == 2u) {
        chunks = (dims.k + dims.chunk - 1u) / dims.chunk;
        chunk = w / tiles;
        k_start = chunk * dims.chunk;
        k_end = min(k_start + dims.chunk, dims.k);
    }
    if (w >= tiles * chunks) { return; }
    let tile = w % tiles;
    let m0 = (tile / tiles_n) * 64u;
    let n0 = (tile % tiles_n) * TN;
    let tr = t / 8u;
    let tc = t % 8u;
    //ACCUMULATORS
    for (var kk = k_start; kk < k_end; kk += 16u) {
        for (var j = 0u; j < 16u; j++) {
            let e = t + 64u * j;
            // A tile: 64 rows (m) x 16 (k).
            if (MODE == 2u) {
                let m = m0 + e % 64u;
                let k = kk + e / 64u;
                tile_a[e] = select(0.0, a[dims.a_off + k * dims.m + m], m < dims.m && k < k_end);
            } else {
                let m = m0 + e / 16u;
                let k = kk + e % 16u;
                tile_a[(e % 16u) * 64u + e / 16u] = select(0.0, a[dims.a_off + m * dims.k + k], m < dims.m && k < k_end);
            }
            // B tile: 16 (k) x TN columns (n).
            if (e < 16u * TN) {
                if (MODE == 0u) {
                    let n = n0 + e / 16u;
                    let k = kk + e % 16u;
                    tile_b[(e % 16u) * TN + e / 16u] = select(0.0, b[dims.b_off + n * dims.k + k], n < dims.n && k < k_end);
                } else {
                    let n = n0 + e % TN;
                    let k = kk + e / TN;
                    tile_b[e] = select(0.0, b[dims.b_off + k * dims.n + n], n < dims.n && k < k_end);
                }
            }
        }
        workgroupBarrier();
        for (var k = 0u; k < 16u; k++) {
            let base = k * 64u;
            let base_b = k * TN;
            //INNER
        }
        workgroupBarrier();
    }
    //STORE
}
