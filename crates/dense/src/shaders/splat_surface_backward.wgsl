// Backward of splat_surface, per interior pixel: from the normal's gradient to the gradients of the two central
// differences (dx along y, dy along x): out[p * 6] = d dx (3), d dy (3); zero elsewhere.

struct Params {
    width: u32, height: u32, pad0: u32, pad1: u32,
    rotation0: vec4<f32>, rotation1: vec4<f32>, rotation2: vec4<f32>,
    intrinsics: vec4<f32>,
    origin: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> pixels: array<f32>;
@group(0) @binding(2) var<storage, read> surface: array<f32>;
@group(0) @binding(3) var<storage, read> dsurface: array<f32>;
@group(0) @binding(4) var<storage, read_write> ddiff: array<f32>;

fn point(x: u32, y: u32) -> vec3<f32> {
    let k = params.intrinsics;
    let c = vec3<f32>((f32(x) - k.z + 0.5) / k.x, (f32(y) - k.w + 0.5) / k.y, 1.0);
    let d = params.rotation0.xyz * c.x + params.rotation1.xyz * c.y + params.rotation2.xyz * c.z;
    return params.origin.xyz + pixels[(y * params.width + x) * 16u + 9u] * d;
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = id.x + id.y * ROW;
    if (p >= params.width * params.height) { return; }
    let x = p % params.width;
    let y = p / params.width;
    for (var k = 0u; k < 6u; k++) { ddiff[p * 6u + k] = 0.0; }
    if (x == 0u || y == 0u || x + 1u >= params.width || y + 1u >= params.height) { return; }
    let dn = vec3<f32>(dsurface[p * 4u], dsurface[p * 4u + 1u], dsurface[p * 4u + 2u]);
    let n = vec3<f32>(surface[p * 8u], surface[p * 8u + 1u], surface[p * 8u + 2u]);
    let len = surface[p * 8u + 6u];
    var dc = dn / 1e-12;
    if (len > 1e-12) { dc = (dn - n * dot(n, dn)) / len; }
    let dx = point(x, y + 1u) - point(x, y - 1u);
    let dy = point(x + 1u, y) - point(x - 1u, y);
    // c = dx x dy: dc . (d dx x dy) = d dx . (dy x dc), dc . (dx x d dy) = d dy . (dc x dx).
    let gdx = cross(dy, dc);
    let gdy = cross(dc, dx);
    ddiff[p * 6u] = gdx.x; ddiff[p * 6u + 1u] = gdx.y; ddiff[p * 6u + 2u] = gdx.z;
    ddiff[p * 6u + 3u] = gdy.x; ddiff[p * 6u + 4u] = gdy.y; ddiff[p * 6u + 5u] = gdy.z;
}
