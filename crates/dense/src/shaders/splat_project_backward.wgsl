// Gradients of the splat parameters from the gradients of their projected values (splat_raster_backward), the
// backward of splat_project: through the screen centre (from the ray transform), the ray transform (K times the
// tangent frame and the centre in camera space), the depth, the normal (sign held), the spherical harmonics (with
// the view direction's dependence on the mean) and the opacity's sigmoid. Hidden splats get nothing. Added to `out`
// (one row of 60 per splat, the parameters' layout).

struct Camera {
    rotation0: vec4<f32>,
    rotation1: vec4<f32>,
    rotation2: vec4<f32>,
    intrinsics: vec4<f32>,
    position: vec4<f32>,
    count: u32, width: u32, height: u32, degree: u32,
    tiles_x: u32, tiles_y: u32, near: f32, far: f32,
}

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<storage, read> splats: array<f32>;
@group(0) @binding(2) var<storage, read> projected: array<f32>;
@group(0) @binding(3) var<storage, read> grads: array<f32>;
@group(0) @binding(4) var<storage, read_write> out: array<f32>;

const STRIDE: u32 = 24u;
const PARAMS: u32 = 60u;
const G: u32 = 20u;

fn rotation(n: vec4<f32>) -> mat3x3<f32> {
    let w = n.x; let x = n.y; let y = n.z; let z = n.w;
    return mat3x3<f32>(
        vec3<f32>(1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y + w * z), 2.0 * (x * z - w * y)),
        vec3<f32>(2.0 * (x * y - w * z), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z + w * x)),
        vec3<f32>(2.0 * (x * z + w * y), 2.0 * (y * z - w * x), 1.0 - 2.0 * (x * x + y * y)),
    );
}

fn g3(base: u32) -> vec3<f32> {
    return vec3<f32>(grads[base], grads[base + 1u], grads[base + 2u]);
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let g = id.x + id.y * ROW;
    if (g >= camera.count) { return; }
    if (projected[g * STRIDE + 19u] <= 0.0) { return; }
    let p = g * PARAMS;
    let gb = g * G;
    let R = transpose(mat3x3<f32>(camera.rotation0.xyz, camera.rotation1.xyz, camera.rotation2.xyz));
    let t = vec3<f32>(camera.rotation0.w, camera.rotation1.w, camera.rotation2.w);
    let mean = vec3<f32>(splats[p], splats[p + 1u], splats[p + 2u]);
    let mean_c = R * mean + t;
    let s = exp(vec3<f32>(splats[p + 7u], splats[p + 8u], splats[p + 9u]));
    let q = vec4<f32>(splats[p + 3u], splats[p + 4u], splats[p + 5u], splats[p + 6u]);
    let qn = q / length(q);
    let rot = rotation(qn);
    let rs_c = R * mat3x3<f32>(rot[0] * s.x, rot[1] * s.y, rot[2] * s.z);
    let k = camera.intrinsics;
    let kmat = mat3x3<f32>(vec3<f32>(k.x, 0.0, 0.0), vec3<f32>(0.0, k.y, 0.0), vec3<f32>(k.z, k.w, 1.0));
    let m0 = kmat * rs_c[0];
    let m1 = kmat * rs_c[1];
    let m2 = kmat * mean_c;
    let u = vec3<f32>(m0.x, m1.x, m2.x);
    let v = vec3<f32>(m0.y, m1.y, m2.y);
    let w = vec3<f32>(m0.z, m1.z, m2.z);
    let test = vec3<f32>(1.0, 1.0, -1.0);
    let d = dot(w * w, test);
    let f = test / d;
    let means2d = vec2<f32>(dot(u * w, f), dot(v * w, f));

    // Gradients of u, v, w: from the rasterizer, plus the screen centre's.
    var du = g3(gb + 7u);
    var dv = g3(gb + 10u);
    var dw = g3(gb + 13u);
    let dxy = vec2<f32>(grads[gb + 16u], grads[gb + 17u]);
    du += dxy.x * w * f;
    dv += dxy.y * w * f;
    dw += dxy.x * (u * f - means2d.x * 2.0 * w * test / d) + dxy.y * (v * f - means2d.y * 2.0 * w * test / d);
    // m0 = (u.x, v.x, w.x), m1 = (u.y, v.y, w.y), m2 = (u.z, v.z, w.z); m = K a, so a's gradient is K^T times m's.
    let dm0 = vec3<f32>(du.x, dv.x, dw.x);
    let dm1 = vec3<f32>(du.y, dv.y, dw.y);
    let dm2 = vec3<f32>(du.z, dv.z, dw.z);
    let kt = transpose(kmat);
    var drs0 = kt * dm0;
    var drs1 = kt * dm1;
    var dmean_c = kt * dm2;
    dmean_c.z += grads[gb + 3u];
    // The normal is the third column turned towards the camera.
    var sign = 1.0;
    if (-dot(rs_c[2], mean_c) <= 0.0) { sign = -1.0; }
    let drs2 = sign * g3(gb + 4u);
    // Camera to world: rs_c = R rs, mean_c = R mean + t.
    let rt = transpose(R);
    let dw0 = rt * drs0;
    let dw1 = rt * drs1;
    let dw2 = rt * drs2;
    var dmean = rt * dmean_c;
    // rs[:, j] = rot[:, j] s_j.
    let ds = vec3<f32>(dot(rot[0], dw0), dot(rot[1], dw1), dot(rot[2], dw2)) * s;
    let dR0 = dw0 * s.x;
    let dR1 = dw1 * s.y;
    let dR2 = dw2 * s.z;
    // dR[i][j] = column j, row i.
    let r00 = dR0.x; let r10 = dR0.y; let r20 = dR0.z;
    let r01 = dR1.x; let r11 = dR1.y; let r21 = dR1.z;
    let r02 = dR2.x; let r12 = dR2.y; let r22 = dR2.z;
    let qw = qn.x; let qx = qn.y; let qy = qn.z; let qz = qn.w;
    let dqn = 2.0 * vec4<f32>(
        -qz * r01 + qy * r02 + qz * r10 - qx * r12 - qy * r20 + qx * r21,
        qy * r01 + qz * r02 + qy * r10 - 2.0 * qx * r11 - qw * r12 + qz * r20 + qw * r21 - 2.0 * qx * r22,
        -2.0 * qy * r00 + qx * r01 + qw * r02 + qx * r10 + qz * r12 - qw * r20 + qz * r21 - 2.0 * qy * r22,
        -2.0 * qz * r00 - qw * r01 + qx * r02 + qw * r10 - 2.0 * qz * r11 + qy * r12 + qx * r20 + qy * r21,
    );
    let dq = (dqn - qn * dot(qn, dqn)) / length(q);

    // Spherical harmonics: coefficients, and the direction (only channels above the clamp at 0).
    let ray = mean - camera.position.xyz;
    let dir = ray / length(ray);
    let x = dir.x; let y = dir.y; let z = dir.z;
    var basis: array<f32, 16>;
    var gx: array<f32, 16>;
    var gy: array<f32, 16>;
    var gz: array<f32, 16>;
    for (var i = 0u; i < 16u; i++) { basis[i] = 0.0; gx[i] = 0.0; gy[i] = 0.0; gz[i] = 0.0; }
    basis[0] = 0.2820947917738781;
    var n = 1u;
    if (camera.degree >= 1u) {
        let a = -0.48860251190292;
        basis[1] = a * y; gy[1] = a;
        basis[2] = -a * z; gz[2] = -a;
        basis[3] = a * x; gx[3] = a;
        n = 4u;
    }
    if (camera.degree >= 2u) {
        let fa = 0.5462742152960395;
        let fb = -1.092548430592079;
        basis[4] = fa * 2.0 * x * y; gx[4] = 2.0 * fa * y; gy[4] = 2.0 * fa * x;
        basis[5] = fb * z * y; gy[5] = fb * z; gz[5] = fb * y;
        basis[6] = 0.9461746957575601 * z * z - 0.3153915652525201; gz[6] = 2.0 * 0.9461746957575601 * z;
        basis[7] = fb * z * x; gx[7] = fb * z; gz[7] = fb * x;
        basis[8] = fa * (x * x - y * y); gx[8] = 2.0 * fa * x; gy[8] = -2.0 * fa * y;
        n = 9u;
    }
    if (camera.degree >= 3u) {
        let ta = -0.5900435899266435;
        let tb = 1.445305721320277;
        let tc = -2.285228997322329 * z * z + 0.4570457994644658;
        let dtc = -2.0 * 2.285228997322329 * z;
        basis[9] = ta * (3.0 * x * x * y - y * y * y); gx[9] = ta * 6.0 * x * y; gy[9] = ta * (3.0 * x * x - 3.0 * y * y);
        basis[10] = tb * z * 2.0 * x * y; gx[10] = tb * 2.0 * y * z; gy[10] = tb * 2.0 * x * z; gz[10] = tb * 2.0 * x * y;
        basis[11] = tc * y; gy[11] = tc; gz[11] = dtc * y;
        basis[12] = z * (1.865881662950577 * z * z - 1.119528997770346); gz[12] = 3.0 * 1.865881662950577 * z * z - 1.119528997770346;
        basis[13] = tc * x; gx[13] = tc; gz[13] = dtc * x;
        basis[14] = tb * z * (x * x - y * y); gx[14] = tb * 2.0 * z * x; gy[14] = -tb * 2.0 * z * y; gz[14] = tb * (x * x - y * y);
        basis[15] = ta * (x * x * x - 3.0 * x * y * y); gx[15] = ta * (3.0 * x * x - 3.0 * y * y); gy[15] = -ta * 6.0 * x * y;
        n = 16u;
    }
    let sh = p + 11u;
    var raw = vec3<f32>(0.0);
    for (var i = 0u; i < n; i++) {
        raw += vec3<f32>(splats[sh + i * 3u], splats[sh + i * 3u + 1u], splats[sh + i * 3u + 2u]) * basis[i];
    }
    let drgb = select(vec3<f32>(0.0), g3(gb), raw + 0.5 > vec3<f32>(0.0));
    var ddir = vec3<f32>(0.0);
    for (var i = 0u; i < n; i++) {
        let c = vec3<f32>(splats[sh + i * 3u], splats[sh + i * 3u + 1u], splats[sh + i * 3u + 2u]);
        out[sh + i * 3u] += basis[i] * drgb.x;
        out[sh + i * 3u + 1u] += basis[i] * drgb.y;
        out[sh + i * 3u + 2u] += basis[i] * drgb.z;
        ddir += dot(c, drgb) * vec3<f32>(gx[i], gy[i], gz[i]);
    }
    dmean += (ddir - dir * dot(dir, ddir)) / length(ray);

    let opacity = 1.0 / (1.0 + exp(-splats[p + 10u]));
    out[p] += dmean.x;
    out[p + 1u] += dmean.y;
    out[p + 2u] += dmean.z;
    out[p + 3u] += dq.x;
    out[p + 4u] += dq.y;
    out[p + 5u] += dq.z;
    out[p + 6u] += dq.w;
    out[p + 7u] += ds.x;
    out[p + 8u] += ds.y;
    out[p + 9u] += ds.z;
    out[p + 10u] += grads[gb + 18u] * opacity * (1.0 - opacity);
}
