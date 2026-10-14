// Input of the colour network per sample: position (3), view direction (3), sin and cos of d_i 2^j (j < 4, 2 x 12,
// i-major), unit normal (3), geometry features (geo - 1). The geometry output has four rows per sample (value and
// three tangents); column 0 is the distance, so the tangent rows' column 0 is its gradient.

struct Params { count: u32, per_ray: u32, geo: u32, colour: u32 }

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> rays: array<f32>;
@group(0) @binding(2) var<storage, read> points: array<f32>;
@group(0) @binding(3) var<storage, read> geometry: array<f32>;
@group(0) @binding(4) var<storage, read_write> x: array<f32>;

const HALF_PI: f32 = 1.5707963267948966;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let s = id.x + id.y * ROW;
    if (s >= params.count) { return; }
    let ray = s / params.per_ray;
    let base = s * params.colour;
    let row = s * 4u * params.geo;
    let d = vec3<f32>(rays[ray * 8u], rays[ray * 8u + 1u], rays[ray * 8u + 2u]);
    let g = vec3<f32>(geometry[row + params.geo], geometry[row + 2u * params.geo], geometry[row + 3u * params.geo]);
    let n = g / max(sqrt(dot(g, g)), 1e-12);
    for (var i = 0u; i < 3u; i++) {
        x[base + i] = points[s * 3u + i];
        x[base + 3u + i] = d[i];
        x[base + 30u + i] = n[i];
        var frequency = 1.0;
        for (var j = 0u; j < 4u; j++) {
            let scaled = d[i] * frequency;
            x[base + 6u + i * 4u + j] = sin(scaled);
            x[base + 18u + i * 4u + j] = sin(scaled + HALF_PI);
            frequency *= 2.0;
        }
    }
    for (var c = 1u; c < params.geo; c++) {
        x[base + 32u + c] = geometry[row + c];
    }
}
