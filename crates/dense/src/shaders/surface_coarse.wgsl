// Even samples of the NeuS sampler: z_i = near + (far - near) (i + offset) / count, and their positions.

struct Params { rays: u32, count: u32, stride: u32, pad: u32 }

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> origins: array<f32>;
@group(0) @binding(2) var<storage, read> rays: array<f32>;
@group(0) @binding(3) var<storage, read_write> depths: array<f32>;
@group(0) @binding(4) var<storage, read_write> points: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let ray = id.x + id.y * ROW;
    if (ray >= params.rays) { return; }
    let o = vec3<f32>(origins[ray * 8u], origins[ray * 8u + 1u], origins[ray * 8u + 2u]);
    let d = vec3<f32>(rays[ray * 8u], rays[ray * 8u + 1u], rays[ray * 8u + 2u]);
    let offset = origins[ray * 8u + 3u];
    let near = origins[ray * 8u + 4u];
    let far = origins[ray * 8u + 5u];
    for (var i = 0u; i < params.count; i++) {
        let z = near + (far - near) * ((f32(i) + offset) / f32(params.count));
        depths[ray * params.stride + i] = z;
        let p = o + z * d;
        let s = ray * params.count + i;
        points[s * 3u] = p.x;
        points[s * 3u + 1u] = p.y;
        points[s * 3u + 2u] = p.z;
    }
}
