// Normals of the rendered median depth (gsplat's depth_to_normal): points o + depth d(x, y) in the world, the cross
// product of the central differences along y and x, normalised; zero on the image border. out[p * 8]: normal (3),
// the unnormalised cross product (3), its length, unused.

struct Params {
    width: u32, height: u32, pad0: u32, pad1: u32,
    rotation0: vec4<f32>, rotation1: vec4<f32>, rotation2: vec4<f32>,   // rows of R, camera = R world + t
    intrinsics: vec4<f32>,
    origin: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> pixels: array<f32>;
@group(0) @binding(2) var<storage, read_write> surface: array<f32>;

fn point(x: u32, y: u32) -> vec3<f32> {
    let k = params.intrinsics;
    let c = vec3<f32>((f32(x) - k.z + 0.5) / k.x, (f32(y) - k.w + 0.5) / k.y, 1.0);
    // R^T c: the columns of R^T are the rows of R.
    let d = params.rotation0.xyz * c.x + params.rotation1.xyz * c.y + params.rotation2.xyz * c.z;
    return params.origin.xyz + pixels[(y * params.width + x) * 16u + 9u] * d;
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = id.x + id.y * ROW;
    if (p >= params.width * params.height) { return; }
    let x = p % params.width;
    let y = p / params.width;
    for (var k = 0u; k < 8u; k++) { surface[p * 8u + k] = 0.0; }
    if (x == 0u || y == 0u || x + 1u >= params.width || y + 1u >= params.height) { return; }
    let dx = point(x, y + 1u) - point(x, y - 1u);
    let dy = point(x + 1u, y) - point(x - 1u, y);
    let c = cross(dx, dy);
    let len = length(c);
    let n = c / max(len, 1e-12);
    surface[p * 8u] = n.x; surface[p * 8u + 1u] = n.y; surface[p * 8u + 2u] = n.z;
    surface[p * 8u + 3u] = c.x; surface[p * 8u + 4u] = c.y; surface[p * 8u + 5u] = c.z;
    surface[p * 8u + 6u] = len;
}
