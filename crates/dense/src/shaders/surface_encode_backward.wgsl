// Gradient of the hash table from the gradient of the encoded input (four rows per point: value and the three
// tangents, as surface_encode writes them). Adds into `grad`, one float per table feature, by compare-and-swap.

struct Params {
    count: u32, tangents: u32, table_size: u32, pad: u32,
    scale: array<vec4<f32>, 4>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> points: array<f32>;
@group(0) @binding(2) var<storage, read> dx: array<f32>;
@group(0) @binding(3) var<storage, read_write> grad: array<atomic<u32>>;

const WIDTH: u32 = 71u;

fn add(index: u32, value: f32) {
    if (value == 0.0) { return; }
    var old = atomicLoad(&grad[index]);
    loop {
        let result = atomicCompareExchangeWeak(&grad[index], old, bitcast<u32>(bitcast<f32>(old) + value));
        if (result.exchanged) { break; }
        old = result.old_value;
    }
}

fn corner_index(cell: vec3<u32>, level: u32) -> u32 {
    let h = (cell.x ^ (cell.y * 2654435761u)) ^ (cell.z * 805459861u);
    return (h & (params.table_size - 1u)) + level * params.table_size;
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let s = id.x + id.y * ROW;
    if (s >= params.count) { return; }
    let p = vec3<f32>(points[s * 3u], points[s * 3u + 1u], points[s * 3u + 2u]);
    let rows = select(1u, 4u, params.tangents == 1u);
    let base = s * rows * WIDTH;
    let u = (p + 2.0) / 4.0;
    for (var level = 0u; level < 16u; level++) {
        let scale = params.scale[level / 4u][level % 4u];
        let scaled = u * scale;
        let low = floor(scaled);
        let f = scaled - low;
        let cell = vec3<u32>(low);
        let column = 39u + level * 2u;
        let g = vec2<f32>(dx[base + column], dx[base + column + 1u]);
        var gx = vec2<f32>(0.0);
        var gy = vec2<f32>(0.0);
        var gz = vec2<f32>(0.0);
        if (rows == 4u) {
            let chain = scale / 4.0;
            gx = vec2<f32>(dx[base + WIDTH + column], dx[base + WIDTH + column + 1u]) * chain;
            gy = vec2<f32>(dx[base + 2u * WIDTH + column], dx[base + 2u * WIDTH + column + 1u]) * chain;
            gz = vec2<f32>(dx[base + 3u * WIDTH + column], dx[base + 3u * WIDTH + column + 1u]) * chain;
        }
        for (var corner = 0u; corner < 8u; corner++) {
            let o = vec3<u32>(corner >> 2u, (corner >> 1u) & 1u, corner & 1u);
            let wx = select(1.0 - f.x, f.x, o.x == 1u);
            let wy = select(1.0 - f.y, f.y, o.y == 1u);
            let wz = select(1.0 - f.z, f.z, o.z == 1u);
            var total = g * (wx * wy * wz);
            if (rows == 4u) {
                let sx = select(-1.0, 1.0, o.x == 1u);
                let sy = select(-1.0, 1.0, o.y == 1u);
                let sz = select(-1.0, 1.0, o.z == 1u);
                total += gx * (sx * wy * wz) + gy * (wx * sy * wz) + gz * (wx * wy * sz);
            }
            let index = corner_index(cell + o, level);
            add(index * 2u, total.x);
            add(index * 2u + 1u, total.y);
        }
    }
}
