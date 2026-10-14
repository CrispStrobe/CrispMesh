// Gradient of the geometry output from the colour network's input gradient: features to the value row's columns
// 1.., the normal's gradient (through n = g / |g|) added to column 0 of the tangent rows, which surface_render
// filled with the distance, cosine and eikonal terms. Columns 1.. of the tangent rows have no gradient.

struct Params { count: u32, per_ray: u32, geo: u32, colour: u32 }

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> dx: array<f32>;
@group(0) @binding(2) var<storage, read> geometry: array<f32>;
@group(0) @binding(3) var<storage, read_write> dgeometry: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let s = id.x + id.y * ROW;
    if (s >= params.count) { return; }
    let base = s * params.colour;
    let row = s * 4u * params.geo;
    let g = vec3<f32>(geometry[row + params.geo], geometry[row + 2u * params.geo], geometry[row + 3u * params.geo]);
    let norm = sqrt(dot(g, g));
    let r = max(norm, 1e-12);
    let dn = vec3<f32>(dx[base + 30u], dx[base + 31u], dx[base + 32u]);
    // n = g / max(|g|, 1e-12): the length's part of the gradient only where it is not clamped.
    var dg = dn / r;
    if (norm > 1e-12) {
        dg -= g * (dot(dn, g) / (r * r * r));
    }
    for (var k = 1u; k < 4u; k++) {
        dgeometry[row + k * params.geo] += dg[k - 1u];
        for (var c = 1u; c < params.geo; c++) {
            dgeometry[row + k * params.geo + c] = 0.0;
        }
    }
    for (var c = 1u; c < params.geo; c++) {
        dgeometry[row + c] = dx[base + 32u + c];
    }
}
