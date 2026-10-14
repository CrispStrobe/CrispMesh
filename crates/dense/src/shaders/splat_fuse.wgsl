// Fusion of one view's rendered median depth into a list of voxels (our trainer's, after the reference run): each
// voxel centre projected to the nearest pixel; where the opacity is below 0.5 or there is no depth the space is
// free (+1); within the truncation in front of the surface or behind it, the clamped distance to it; further
// behind, counted as behind. Per voxel: total, weight, behind.

struct Params {
    count: u32, resolution: u32, width: u32, height: u32,
    rotation0: vec4<f32>, rotation1: vec4<f32>, rotation2: vec4<f32>,   // rows of R with t in w
    intrinsics: vec4<f32>,
    truncation: f32, pad0: f32, pad1: f32, pad2: f32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> voxels: array<u32>;
@group(0) @binding(2) var<storage, read> pixels: array<f32>;
@group(0) @binding(3) var<storage, read_write> sums: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x + id.y * ROW;
    if (i >= params.count) { return; }
    let index = voxels[i];
    let r = params.resolution;
    let size = 2.0 / f32(r);
    let p = vec3<f32>(f32(index / (r * r)), f32((index / r) % r), f32(index % r)) * size + 0.5 * size - 1.0;
    let c = vec3<f32>(dot(params.rotation0.xyz, p) + params.rotation0.w, dot(params.rotation1.xyz, p) + params.rotation1.w, dot(params.rotation2.xyz, p) + params.rotation2.w);
    if (c.z <= 0.0) { return; }
    let k = params.intrinsics;
    let u = round(k.x * c.x / c.z + k.z - 0.5);
    let v = round(k.y * c.y / c.z + k.w - 0.5);
    if (u < 0.0 || v < 0.0 || u >= f32(params.width) || v >= f32(params.height)) { return; }
    let pixel = u32(v) * params.width + u32(u);
    let depth = pixels[pixel * 16u + 9u];
    let opacity = pixels[pixel * 16u + 3u];
    let sdf = depth - c.z;
    let free = depth <= 0.0 || opacity < 0.5;
    let near = !free && sdf > -params.truncation;
    if (free || near) {
        sums[i * 3u] += select(clamp(sdf / params.truncation, -1.0, 1.0), 1.0, free);
        sums[i * 3u + 1u] += 1.0;
    } else {
        sums[i * 3u + 2u] += 1.0;
    }
}
