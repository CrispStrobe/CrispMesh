// What the backward rasterizer needs per pixel, packed into one 64-byte row (four vec4): the image gradients of
// colour (3) and camera normal (3), their dot products with the rendered colour and normal sums, transmittance times
// the opacity's gradient, the distortion's gradient, the depth and opacity sums, four times the distortion total,
// the last splat (+ 1), the median splat (+ 1) and the median depth's gradient.

struct Params { count: u32, pad0: u32, pad1: u32, pad2: u32 }

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> pixels: array<f32>;
@group(0) @binding(2) var<storage, read> dpixels: array<f32>;
@group(0) @binding(3) var<storage, read_write> packed: array<vec4<f32>>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = id.x + id.y * ROW;
    if (p >= params.count) { return; }
    let o = p * 16u;
    let d = p * 12u;
    let dc = vec3<f32>(dpixels[d], dpixels[d + 1u], dpixels[d + 2u]);
    let dn = vec3<f32>(dpixels[d + 4u], dpixels[d + 5u], dpixels[d + 6u]);
    let colour = vec3<f32>(pixels[o], pixels[o + 1u], pixels[o + 2u]);
    let normal = vec3<f32>(pixels[o + 4u], pixels[o + 5u], pixels[o + 6u]);
    packed[p * 4u] = vec4<f32>(dc, dn.x);
    packed[p * 4u + 1u] = vec4<f32>(dn.y, dn.z, dot(colour, dc), dot(normal, dn));
    packed[p * 4u + 2u] = vec4<f32>(pixels[o + 10u] * dpixels[d + 3u], dpixels[d + 7u], pixels[o + 7u], pixels[o + 3u]);
    packed[p * 4u + 3u] = vec4<f32>(4.0 * pixels[o + 13u], pixels[o + 11u], pixels[o + 12u], dpixels[d + 8u]);
}
