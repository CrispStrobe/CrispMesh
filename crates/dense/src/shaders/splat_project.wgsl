// Projection of 2D Gaussian splats into one camera, after gsplat's 2DGS projection (Apache-2.0): the ray transform
// M (the splat's tangent frame and centre through K, transposed), the screen centre and extent (radius 3.33 sigma),
// the normal turned towards the camera, the colour from spherical harmonics (degree <= 3, plus 0.5, at least 0),
// the opacity, the covered rectangle of 16-pixel tiles and the depth sort key (all ones for hidden splats).
// Per splat out: means2d (2), depth, the rasterizer's u, v, w rows (9), normal (3, camera), colour (3), opacity, radius (2), tile
// rectangle (4: x0, y0, x1, y1, exclusive), tile count, sort key.

struct Camera {
    rotation0: vec4<f32>,      // rows of R with t in w: camera = R world + t
    rotation1: vec4<f32>,
    rotation2: vec4<f32>,
    intrinsics: vec4<f32>,     // fx, fy, cx, cy
    position: vec4<f32>,       // camera centre in the world
    count: u32, width: u32, height: u32, degree: u32,
    tiles_x: u32, tiles_y: u32, near: f32, far: f32,
}

@group(0) @binding(0) var<uniform> camera: Camera;
// Per splat in `splats` (stride 60): mean (3), quaternion w, x, y, z (4), log scales (3), logit opacity, spherical
// harmonics (16 x 3), unused.
@group(0) @binding(1) var<storage, read> splats: array<f32>;
@group(0) @binding(2) var<storage, read_write> projected: array<f32>;
@group(0) @binding(3) var<storage, read_write> sort: array<u32>;

const STRIDE: u32 = 24u;
const PARAMS: u32 = 60u;
const TILE: f32 = 16.0;

fn rotation(q: vec4<f32>) -> mat3x3<f32> {
    let n = q / length(q);
    let w = n.x; let x = n.y; let y = n.z; let z = n.w;
    // Columns of the matrix whose rows gsplat writes.
    return mat3x3<f32>(
        vec3<f32>(1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y + w * z), 2.0 * (x * z - w * y)),
        vec3<f32>(2.0 * (x * y - w * z), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z + w * x)),
        vec3<f32>(2.0 * (x * z + w * y), 2.0 * (y * z - w * x), 1.0 - 2.0 * (x * x + y * y)),
    );
}

fn sh_colour(g: u32, dir: vec3<f32>) -> vec3<f32> {
    let base = g * PARAMS + 11u;
    var c = vec3<f32>(splats[base], splats[base + 1u], splats[base + 2u]) * 0.2820947917738781;
    if (camera.degree >= 1u) {
        let x = dir.x; let y = dir.y; let z = dir.z;
        let a = -0.48860251190292;
        var b: array<f32, 16>;
        b[1] = a * y; b[2] = -a * z; b[3] = a * x;
        var n = 4u;
        if (camera.degree >= 2u) {
            let z2 = z * z; let fb = -1.092548430592079 * z; let fa = 0.5462742152960395;
            let c1 = x * x - y * y; let s1 = 2.0 * x * y;
            b[4] = fa * s1; b[5] = fb * y; b[6] = 0.9461746957575601 * z2 - 0.3153915652525201; b[7] = fb * x; b[8] = fa * c1;
            n = 9u;
            if (camera.degree >= 3u) {
                let tc = -2.285228997322329 * z2 + 0.4570457994644658; let tb = 1.445305721320277 * z; let ta = -0.5900435899266435;
                let c2 = x * c1 - y * s1; let s2 = x * s1 + y * c1;
                b[9] = ta * s2; b[10] = tb * s1; b[11] = tc * y; b[12] = z * (1.865881662950577 * z2 - 1.119528997770346);
                b[13] = tc * x; b[14] = tb * c1; b[15] = ta * c2;
                n = 16u;
            }
        }
        for (var k = 1u; k < n; k++) {
            c += vec3<f32>(splats[base + k * 3u], splats[base + k * 3u + 1u], splats[base + k * 3u + 2u]) * b[k];
        }
    }
    return max(c + 0.5, vec3<f32>(0.0));
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let g = id.x + id.y * ROW;
    if (g >= camera.count) { return; }
    let R = transpose(mat3x3<f32>(camera.rotation0.xyz, camera.rotation1.xyz, camera.rotation2.xyz));
    let t = vec3<f32>(camera.rotation0.w, camera.rotation1.w, camera.rotation2.w);
    let p = g * PARAMS;
    let mean = vec3<f32>(splats[p], splats[p + 1u], splats[p + 2u]);
    let mean_c = R * mean + t;
    let s = exp(vec3<f32>(splats[p + 7u], splats[p + 8u], splats[p + 9u]));
    let rs = rotation(vec4<f32>(splats[p + 3u], splats[p + 4u], splats[p + 5u], splats[p + 6u]));
    // The third column unscaled (gsplat's CUDA projection): the normal is a unit vector.
    let rs_c = R * mat3x3<f32>(rs[0] * s.x, rs[1] * s.y, rs[2]);
    var normal = rs_c[2];
    if (-dot(normal, mean_c) <= 0.0) { normal = -normal; }
    // T = K [RS_c[:, 0], RS_c[:, 1], mean_c]; M = T^T, so the rows of M are the columns of T.
    let k = camera.intrinsics;
    let kmat = mat3x3<f32>(vec3<f32>(k.x, 0.0, 0.0), vec3<f32>(0.0, k.y, 0.0), vec3<f32>(k.z, k.w, 1.0));
    let m0 = kmat * rs_c[0];
    let m1 = kmat * rs_c[1];
    let m2 = kmat * mean_c;
    // M[..., i] in gsplat (columns of M) are (m0[i], m1[i], m2[i]).
    let col0 = vec3<f32>(m0.x, m1.x, m2.x);
    let col1 = vec3<f32>(m0.y, m1.y, m2.y);
    let col2 = vec3<f32>(m0.z, m1.z, m2.z);
    let test = vec3<f32>(1.0, 1.0, -1.0);
    let d = dot(col2 * col2, test);
    var shown = d != 0.0;
    var f = vec3<f32>(0.0);
    if (shown) { f = test / d; }
    let means2d = vec2<f32>(dot(col0 * col2, f), dot(col1 * col2, f));
    let extent = sqrt(max(means2d * means2d - vec2<f32>(dot(col0 * col0, f), dot(col1 * col1, f)), vec2<f32>(1e-4)));
    let depth = mean_c.z;
    var radius = ceil(3.33 * extent);
    shown = shown && depth > camera.near && depth < camera.far;
    shown = shown && means2d.x + radius.x > 0.0 && means2d.x - radius.x < f32(camera.width) && means2d.y + radius.y > 0.0 && means2d.y - radius.y < f32(camera.height);
    if (!shown) { radius = vec2<f32>(0.0); }
    let dir = normalize(mean - camera.position.xyz);
    let colour = sh_colour(g, dir);
    let base = g * STRIDE;
    projected[base] = means2d.x;
    projected[base + 1u] = means2d.y;
    projected[base + 2u] = depth;
    // gsplat hands the rasterizer M transposed back (K T): its rows u, v, w are the columns of M.
    projected[base + 3u] = col0.x; projected[base + 4u] = col0.y; projected[base + 5u] = col0.z;
    projected[base + 6u] = col1.x; projected[base + 7u] = col1.y; projected[base + 8u] = col1.z;
    projected[base + 9u] = col2.x; projected[base + 10u] = col2.y; projected[base + 11u] = col2.z;
    projected[base + 12u] = normal.x; projected[base + 13u] = normal.y; projected[base + 14u] = normal.z;
    projected[base + 15u] = colour.x; projected[base + 16u] = colour.y; projected[base + 17u] = colour.z;
    projected[base + 18u] = 1.0 / (1.0 + exp(-splats[p + 10u]));
    projected[base + 19u] = radius.x;
    projected[base + 20u] = radius.y;
    let tiles = vec2<f32>(f32(camera.tiles_x), f32(camera.tiles_y));
    let tile_mean = means2d / TILE;
    let tile_radius = radius / TILE;
    let lo = vec2<u32>(clamp(floor(tile_mean - tile_radius), vec2<f32>(0.0), tiles));
    let hi = vec2<u32>(clamp(ceil(tile_mean + tile_radius), vec2<f32>(0.0), tiles));
    var count = (hi.x - lo.x) * (hi.y - lo.y);
    if (!shown) { count = 0u; }
    projected[base + 21u] = bitcast<f32>(lo.x | (lo.y << 16u));
    projected[base + 22u] = bitcast<f32>(hi.x | (hi.y << 16u));
    projected[base + 23u] = bitcast<f32>(count);
    // Depth keys of shown splats are positive floats, whose bits order like their values.
    sort[g] = select(0xffffffffu, bitcast<u32>(depth), shown);
}
