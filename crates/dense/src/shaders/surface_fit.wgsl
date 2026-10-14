// Gradient of fitting the distance to known targets: mean |distance - target| + eikonal mean (|gradient| - 1)^2,
// over `1 / inv_count` points in all passes. Writes the whole gradient of the geometry output (four rows per point)
// and each point's loss.

struct Params { count: u32, geo: u32, inv_count: f32, eikonal: f32 }

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> targets: array<f32>;
@group(0) @binding(2) var<storage, read> geometry: array<f32>;
@group(0) @binding(3) var<storage, read_write> dgeometry: array<f32>;
@group(0) @binding(4) var<storage, read_write> losses: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let s = id.x + id.y * ROW;
    if (s >= params.count) { return; }
    let row = s * 4u * params.geo;
    let g = vec3<f32>(geometry[row + params.geo], geometry[row + 2u * params.geo], geometry[row + 3u * params.geo]);
    let difference = geometry[row] - targets[s];
    let norm = sqrt(dot(g, g) + 1e-12);
    for (var k = 0u; k < 4u; k++) {
        for (var c = 1u; c < params.geo; c++) {
            dgeometry[row + k * params.geo + c] = 0.0;
        }
    }
    dgeometry[row] = sign(difference) * params.inv_count;
    let dg = g * (params.eikonal * params.inv_count * 2.0 * (norm - 1.0) / norm);
    dgeometry[row + params.geo] = dg.x;
    dgeometry[row + 2u * params.geo] = dg.y;
    dgeometry[row + 3u * params.geo] = dg.z;
    losses[s] = abs(difference) + params.eikonal * (norm - 1.0) * (norm - 1.0);
}
