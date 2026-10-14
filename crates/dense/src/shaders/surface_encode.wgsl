// Input of the geometry network for each point p (unit-sphere coordinates): p, sin and cos of p_i 2^j (j < 6, 2 x 18
// values, i-major), and 16 levels x 2 features of a hash grid over (p + 2) / 4 with trilinear interpolation (71 values).
// With `tangents` each point has four rows: the values, then their derivatives along x, y and z.

struct Params {
    count: u32, tangents: u32, table_size: u32, pad: u32,
    scale: array<vec4<f32>, 4>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> points: array<f32>;
@group(0) @binding(2) var<storage, read> table: array<f32>;
@group(0) @binding(3) var<storage, read_write> x: array<f32>;

const WIDTH: u32 = 71u;
const HALF_PI: f32 = 1.5707963267948966;

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
    for (var i = 0u; i < 3u; i++) {
        x[base + i] = p[i];
        if (rows == 4u) {
            for (var k = 0u; k < 3u; k++) { x[base + (k + 1u) * WIDTH + i] = select(0.0, 1.0, i == k); }
        }
    }
    for (var i = 0u; i < 3u; i++) {
        var frequency = 1.0;
        for (var j = 0u; j < 6u; j++) {
            let scaled = p[i] * frequency;
            x[base + 3u + i * 6u + j] = sin(scaled);
            x[base + 21u + i * 6u + j] = sin(scaled + HALF_PI);
            if (rows == 4u) {
                for (var k = 0u; k < 3u; k++) {
                    x[base + (k + 1u) * WIDTH + 3u + i * 6u + j] = select(0.0, frequency * cos(scaled), i == k);
                    x[base + (k + 1u) * WIDTH + 21u + i * 6u + j] = select(0.0, frequency * cos(scaled + HALF_PI), i == k);
                }
            }
            frequency *= 2.0;
        }
    }
    let u = (p + 2.0) / 4.0;
    for (var level = 0u; level < 16u; level++) {
        let scale = params.scale[level / 4u][level % 4u];
        let scaled = u * scale;
        let low = floor(scaled);
        let f = scaled - low;
        let cell = vec3<u32>(low);
        var value = vec2<f32>(0.0);
        var dx = vec2<f32>(0.0);
        var dy = vec2<f32>(0.0);
        var dz = vec2<f32>(0.0);
        for (var corner = 0u; corner < 8u; corner++) {
            let o = vec3<u32>(corner >> 2u, (corner >> 1u) & 1u, corner & 1u);
            let wx = select(1.0 - f.x, f.x, o.x == 1u);
            let wy = select(1.0 - f.y, f.y, o.y == 1u);
            let wz = select(1.0 - f.z, f.z, o.z == 1u);
            let index = corner_index(cell + o, level);
            let entry = vec2<f32>(table[index * 2u], table[index * 2u + 1u]);
            value += entry * (wx * wy * wz);
            if (rows == 4u) {
                let sx = select(-1.0, 1.0, o.x == 1u);
                let sy = select(-1.0, 1.0, o.y == 1u);
                let sz = select(-1.0, 1.0, o.z == 1u);
                dx += entry * (sx * wy * wz);
                dy += entry * (wx * sy * wz);
                dz += entry * (wx * wy * sz);
            }
        }
        let column = 39u + level * 2u;
        x[base + column] = value.x;
        x[base + column + 1u] = value.y;
        if (rows == 4u) {
            let chain = scale / 4.0;
            x[base + WIDTH + column] = dx.x * chain;
            x[base + WIDTH + column + 1u] = dx.y * chain;
            x[base + 2u * WIDTH + column] = dy.x * chain;
            x[base + 2u * WIDTH + column + 1u] = dy.y * chain;
            x[base + 3u * WIDTH + column] = dz.x * chain;
            x[base + 3u * WIDTH + column + 1u] = dz.y * chain;
        }
    }
}
