// Densification statistics of one view, as gsplat's DefaultStrategy with key "gradient_2dgs": for each shown splat,
// the norm of (d/du_z, d/dv_z) times its depth, scaled by width / 2 and height / 2, is added to `stats[g * 2]` and
// the view counted in `stats[g * 2 + 1]`. (gsplat writes that gradient from inside its backward rasterizer, racing;
// here it is taken from the finished gradients.)

struct Params { count: u32, width: u32, height: u32, pad: u32 }

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> projected: array<f32>;
@group(0) @binding(2) var<storage, read> grads: array<f32>;
@group(0) @binding(3) var<storage, read_write> stats: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let g = id.x + id.y * ROW;
    if (g >= params.count) { return; }
    if (projected[g * 24u + 19u] <= 0.0 || projected[g * 24u + 20u] <= 0.0) { return; }
    let depth = projected[g * 24u + 11u];        // w_z of the ray transform: the depth
    let gx = grads[g * 20u + 9u] * depth * f32(params.width) / 2.0;
    let gy = grads[g * 20u + 12u] * depth * f32(params.height) / 2.0;
    stats[g * 2u] += sqrt(gx * gx + gy * gy);
    stats[g * 2u + 1u] += 1.0;
}
