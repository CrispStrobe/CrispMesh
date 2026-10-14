// Gaussian blur along x (axis 0) or y (axis 1) of `planes` interleaved maps (pixel-major), 11 taps, sigma 1.5,
// zero outside the image (as a convolution with zero padding).

struct Params { width: u32, height: u32, planes: u32, axis: u32 }

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> source: array<f32>;
@group(0) @binding(2) var<storage, read_write> target_maps: array<f32>;

fn weight(k: i32) -> f32 {
    let x = f32(k);
    return exp(-(x * x) / (2.0 * 1.5 * 1.5));
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let e = id.x + id.y * ROW;
    if (e >= params.width * params.height * params.planes) { return; }
    let p = e / params.planes;
    let c = e % params.planes;
    let x = i32(p % params.width);
    let y = i32(p / params.width);
    var norm = 0.0;
    for (var k = -5; k <= 5; k++) { norm += weight(k); }
    var total = 0.0;
    for (var k = -5; k <= 5; k++) {
        var xx = x;
        var yy = y;
        if (params.axis == 0u) { xx = x + k; } else { yy = y + k; }
        if (xx < 0 || yy < 0 || xx >= i32(params.width) || yy >= i32(params.height)) { continue; }
        total += weight(k) * source[(u32(yy) * params.width + u32(xx)) * params.planes + c];
    }
    target_maps[e] = total / norm;
}
