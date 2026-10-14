// Ray limits: the part inside the unit sphere (near >= 0.05), and with `band` > 0 only where the hull's signed
// distance (a grid over [-1, 1]^3, read trilinearly at 257 points) is below `band`. Rays that miss get [far, far + 1e-4].
// Per ray in `origins`: origin (3), stratified offset, near, far (written here), unused (2).

struct Params { rays: u32, resolution: u32, band: f32, pad: u32 }

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> origins: array<f32>;
@group(0) @binding(2) var<storage, read> rays: array<f32>;
@group(0) @binding(3) var<storage, read> grid: array<f32>;

fn hull(p: vec3<f32>) -> f32 {
    let r = params.resolution;
    let g = clamp((p + 1.0) * (0.5 * f32(r - 1u)), vec3<f32>(0.0), vec3<f32>(f32(r) - 1.001));
    let i = floor(g);
    let f = g - i;
    let c = vec3<u32>(i);
    var total = 0.0;
    for (var corner = 0u; corner < 8u; corner++) {
        let o = vec3<u32>(corner >> 2u, (corner >> 1u) & 1u, corner & 1u);
        let w = select(1.0 - f.x, f.x, o.x == 1u) * select(1.0 - f.y, f.y, o.y == 1u) * select(1.0 - f.z, f.z, o.z == 1u);
        total += grid[((c.x + o.x) * r + (c.y + o.y)) * r + (c.z + o.z)] * w;
    }
    return total;
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let ray = id.x + id.y * ROW;
    if (ray >= params.rays) { return; }
    let o = vec3<f32>(origins[ray * 8u], origins[ray * 8u + 1u], origins[ray * 8u + 2u]);
    let d = vec3<f32>(rays[ray * 8u], rays[ray * 8u + 1u], rays[ray * 8u + 2u]);
    let b = dot(o, d);
    let c = dot(o, o) - 1.0;
    let root = sqrt(max(b * b - c, 1e-6));
    var near = max(-b - root, 0.05);
    var far = max(-b + root, 0.06);
    if (params.band > 0.0) {
        var first = 257u;
        var last = 0u;
        var any = false;
        for (var j = 0u; j < 257u; j++) {
            let t = near + (far - near) * (f32(j) / 256.0);
            if (hull(o + t * d) < params.band) {
                first = min(first, j);
                last = j;
                any = true;
            }
        }
        let step = (far - near) / 256.0;
        if (any) {
            let new_near = near + f32(max(i32(first) - 1, 0)) * step;
            far = near + f32(min(last + 1u, 256u)) * step;
            near = new_near;
        } else {
            near = far;
            far = far + 1e-4;
        }
    }
    origins[ray * 8u + 4u] = near;
    origins[ray * 8u + 5u] = far;
}
