// Per pixel and colour channel: the target a (photo inside the mask, the background outside) and the rendering b
// (colour sum plus the background behind), and the five maps SSIM blurs: a, b, a^2, b^2, a b (out[p * 15 + c * 5 + k]).
// The photo arrives as RGBA8 with the mask in alpha.

struct Params { width: u32, height: u32, pad0: u32, pad1: u32, background: vec4<f32> }

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> pixels: array<f32>;
@group(0) @binding(2) var<storage, read> photo: array<u32>;
@group(0) @binding(3) var<storage, read_write> maps: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = id.x + id.y * ROW;
    if (p >= params.width * params.height) { return; }
    let rgba = unpack4x8unorm(photo[p]);
    let mask = select(0.0, 1.0, rgba.w > 0.5);
    let alpha = pixels[p * 16u + 3u];
    for (var c = 0u; c < 3u; c++) {
        let a = rgba[c] * mask + params.background[c] * (1.0 - mask);
        let b = pixels[p * 16u + c] + params.background[c] * (1.0 - alpha);
        let o = p * 15u + c * 5u;
        maps[o] = a;
        maps[o + 1u] = b;
        maps[o + 2u] = a * a;
        maps[o + 3u] = b * b;
        maps[o + 4u] = a * b;
    }
}
