// The median depth's gradient per pixel from the central differences' gradients of its four neighbours (each
// difference adds the point below or right and subtracts the one above or left), along the pixel's ray.

struct Params {
    width: u32, height: u32, pad0: u32, pad1: u32,
    rotation0: vec4<f32>, rotation1: vec4<f32>, rotation2: vec4<f32>,
    intrinsics: vec4<f32>,
    origin: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> ddiff: array<f32>;
@group(0) @binding(2) var<storage, read_write> dpixels: array<f32>;

fn at(q: u32, k: u32) -> vec3<f32> {
    return vec3<f32>(ddiff[q * 6u + k], ddiff[q * 6u + k + 1u], ddiff[q * 6u + k + 2u]);
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = id.x + id.y * ROW;
    if (p >= params.width * params.height) { return; }
    let x = p % params.width;
    let y = p / params.width;
    let w = params.width;
    var g = vec3<f32>(0.0);
    if (y >= 1u) { g += at(p - w, 0u); }                       // this point is the "below" of the pixel above
    if (y + 1u < params.height) { g -= at(p + w, 0u); }
    if (x >= 1u) { g += at(p - 1u, 3u); }
    if (x + 1u < params.width) { g -= at(p + 1u, 3u); }
    let k = params.intrinsics;
    let c = vec3<f32>((f32(x) - k.z + 0.5) / k.x, (f32(y) - k.w + 0.5) / k.y, 1.0);
    let d = params.rotation0.xyz * c.x + params.rotation1.xyz * c.y + params.rotation2.xyz * c.z;
    dpixels[p * 12u + 8u] = dot(g, d);
}
