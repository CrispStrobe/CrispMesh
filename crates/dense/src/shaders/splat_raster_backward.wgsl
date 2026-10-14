// Gradients of the splats' projected values from the gradients of the rendered images. The formulas are gsplat's
// RasterizeToPixels2DGSBwd (Apache-2.0); the schedule is Brush's (Apache-2.0) per-splat backward: one workgroup of
// 32 invocations per tile, each owning one splat of the current batch, the tile's pixels replayed front to back
// along a diagonal (at step i invocation t handles pixel i - t, so each pixel meets the splats in order), the
// pixels' running state in workgroup memory, each splat's gradient summed in its invocation and added to memory once
// per batch (compare-and-swap). What lies behind a splat comes from the forward pass's totals minus what lies in
// front, so transmittance is never divided back.
// Per pixel in (stride 12): colour (3), alpha, normal sum in camera space (3), distortion, median depth, unused (3).
// Per splat out (stride 20): colour (3), depth, normal (3), u, v, w rows of the ray transform (9), means2d (2),
// opacity (after the sigmoid), unused.

struct Params { width: u32, height: u32, tiles_x: u32, pad: u32 }

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> ranges: array<u32>;
@group(0) @binding(2) var<storage, read> values: array<u32>;
@group(0) @binding(3) var<storage, read> projected: array<f32>;
@group(0) @binding(4) var<storage, read> pixels: array<f32>;
@group(0) @binding(5) var<storage, read> dpixels: array<f32>;
@group(0) @binding(6) var<storage, read_write> grads: array<atomic<u32>>;

const STRIDE: u32 = 24u;
const OUT: u32 = 16u;
const DIN: u32 = 12u;
const G: u32 = 20u;
const BATCH: u32 = 32u;
const PIXELS: u32 = 256u;

// Per tile pixel: transmittance before the next splat, and the sums over the splats in front of it of colour (3) and
// normal (3) times their weight, of weight times depth, of weight, and of the distortion's dL/dw times weight.
var<workgroup> state_T: array<f32, 256>;
var<workgroup> front_colour: array<vec3<f32>, 256>;
var<workgroup> front_normal: array<vec3<f32>, 256>;
var<workgroup> front_d: array<f32, 256>;
var<workgroup> front_w: array<f32, 256>;
var<workgroup> front_distort: array<f32, 256>;
var<workgroup> furthest: atomic<u32>;
var<workgroup> range: array<u32, 2>;

fn read3(base: u32) -> vec3<f32> {
    return vec3<f32>(projected[base], projected[base + 1u], projected[base + 2u]);
}

fn add(index: u32, value: f32) {
    if (value == 0.0) { return; }
    var old = atomicLoad(&grads[index]);
    loop {
        let result = atomicCompareExchangeWeak(&grads[index], old, bitcast<u32>(bitcast<f32>(old) + value));
        if (result.exchanged) { break; }
        old = result.old_value;
    }
}

@compute @workgroup_size(32)
fn main(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let tile = group.x + group.y * (ROW / 64u);
    let tiles_y = (params.height + 15u) / 16u;
    if (tile >= params.tiles_x * tiles_y) { return; }
    let tx = tile % params.tiles_x;
    let ty = tile / params.tiles_x;
    if (t == 0u) {
        range[0] = ranges[tile * 2u];
        range[1] = ranges[tile * 2u + 1u];
        atomicStore(&furthest, 0u);
    }
    workgroupBarrier();
    for (var k = t; k < PIXELS; k += BATCH) {
        state_T[k] = 1.0;
        front_colour[k] = vec3<f32>(0.0);
        front_normal[k] = vec3<f32>(0.0);
        front_d[k] = 0.0;
        front_w[k] = 0.0;
        front_distort[k] = 0.0;
        let x = tx * 16u + k % 16u;
        let y = ty * 16u + k / 16u;
        if (x < params.width && y < params.height) {
            atomicMax(&furthest, bitcast<u32>(pixels[(y * params.width + x) * OUT + 11u]));
        }
    }
    let start = workgroupUniformLoad(&range[0]);
    // Splats behind every pixel's last one contribute nothing.
    let end = min(workgroupUniformLoad(&range[1]), workgroupUniformLoad(&furthest));
    var batches = 0u;
    if (end > start) { batches = (end - start + BATCH - 1u) / BATCH; }
    for (var b = 0u; b < batches; b++) {
        let index = start + b * BATCH + t;
        let live = index < end;
        var g = 0u;
        var xy = vec2<f32>(0.0);
        var opacity = 0.0;
        var u = vec3<f32>(0.0);
        var v = vec3<f32>(0.0);
        var w = vec3<f32>(0.0);
        var colour = vec3<f32>(0.0);
        var depth = 0.0;
        var normal = vec3<f32>(0.0);
        if (live) {
            g = values[index];
            let base = g * STRIDE;
            xy = vec2<f32>(projected[base], projected[base + 1u]);
            depth = projected[base + 2u];
            u = read3(base + 3u);
            v = read3(base + 6u);
            w = read3(base + 9u);
            normal = read3(base + 12u);
            colour = read3(base + 15u);
            opacity = projected[base + 18u];
        }
        var d_colour = vec3<f32>(0.0);
        var d_depth = 0.0;
        var d_normal = vec3<f32>(0.0);
        var d_u = vec3<f32>(0.0);
        var d_v = vec3<f32>(0.0);
        var d_w = vec3<f32>(0.0);
        var d_xy = vec2<f32>(0.0);
        var d_opacity = 0.0;
        for (var step = 0u; step < PIXELS + BATCH - 1u; step++) {
            let k = step - t;                         // wraps past PIXELS when step < t
            if (live && step >= t && k < PIXELS) {
                let x = tx * 16u + k % 16u;
                let y = ty * 16u + k / 16u;
                let p = y * params.width + x;
                let at = index + 1u;
                if (x < params.width && y < params.height && at <= bitcast<u32>(pixels[p * OUT + 11u])) {
                    let px = f32(x) + 0.5;
                    let py = f32(y) + 0.5;
                    let h_u = px * w - u;
                    let h_v = py * w - v;
                    let c = cross(h_u, h_v);
                    let dx = xy.x - px;
                    let dy = xy.y - py;
                    let sx = c.x / select(1.0, c.z, c.z != 0.0);
                    let sy = c.y / select(1.0, c.z, c.z != 0.0);
                    let weight_3d = sx * sx + sy * sy;
                    let weight_2d = 2.0 * (dx * dx + dy * dy);
                    let sigma = 0.5 * min(weight_3d, weight_2d);
                    let vis = exp(-sigma);
                    let alpha = min(0.999, opacity * vis);
                    if (c.z != 0.0 && sigma >= 0.0 && alpha >= 1.0 / 255.0) {
                        let T = state_T[k];
                        let fac = alpha * T;
                        let ra = 1.0 / (1.0 - alpha);
                        let o = p * OUT;
                        let di = p * DIN;
                        let dc = vec3<f32>(dpixels[di], dpixels[di + 1u], dpixels[di + 2u]);
                        let da = dpixels[di + 3u];
                        let dn = vec3<f32>(dpixels[di + 4u], dpixels[di + 5u], dpixels[di + 6u]);
                        let ddistort = dpixels[di + 7u];
                        let T_final = pixels[o + 10u];
                        let total_w = pixels[o + 3u];
                        let total_d = pixels[o + 7u];
                        // What lies behind this splat: the totals minus the front and this splat.
                        let behind_colour = vec3<f32>(pixels[o], pixels[o + 1u], pixels[o + 2u]) - front_colour[k] - colour * fac;
                        let behind_normal = vec3<f32>(pixels[o + 4u], pixels[o + 5u], pixels[o + 6u]) - front_normal[k] - normal * fac;
                        var v_alpha = dot(colour * T - behind_colour * ra, dc) + dot(normal * T - behind_normal * ra, dn) + T_final * ra * da;
                        let w_upto = front_w[k] + fac;
                        let d_upto = front_d[k] + fac * depth;
                        let dl_dw = 2.0 * (2.0 * (depth * w_upto - d_upto) + (total_d - depth * total_w));
                        let distort_behind = 4.0 * pixels[o + 13u] - (front_distort[k] + dl_dw * fac);
                        v_alpha += (dl_dw * T - distort_behind * ra) * ddistort;
                        d_depth += 2.0 * fac * (2.0 - 2.0 * T - total_w + fac) * ddistort;
                        if (bitcast<u32>(pixels[o + 12u]) == at) { d_depth += dpixels[di + 8u]; }
                        d_colour += fac * dc;
                        d_normal += fac * dn;
                        if (opacity * vis <= 0.999) {
                            let v_G = opacity * v_alpha;
                            if (weight_3d <= weight_2d) {
                                let vsx = v_G * -vis * sx / c.z;
                                let vsy = v_G * -vis * sy / c.z;
                                let v_cross = vec3<f32>(vsx, vsy, -(vsx * sx + vsy * sy));
                                let v_h_u = cross(h_v, v_cross);
                                let v_h_v = cross(v_cross, h_u);
                                d_u -= v_h_u;
                                d_v -= v_h_v;
                                d_w += px * v_h_u + py * v_h_v;
                            } else {
                                d_xy += v_G * -vis * 2.0 * vec2<f32>(dx, dy);
                            }
                            d_opacity += vis * v_alpha;
                        }
                        state_T[k] = T * (1.0 - alpha);
                        front_colour[k] += colour * fac;
                        front_normal[k] += normal * fac;
                        front_d[k] = d_upto;
                        front_w[k] = w_upto;
                        front_distort[k] += dl_dw * fac;
                    }
                }
            }
            workgroupBarrier();
        }
        if (live) {
            let base = g * G;
            add(base, d_colour.x); add(base + 1u, d_colour.y); add(base + 2u, d_colour.z);
            add(base + 3u, d_depth);
            add(base + 4u, d_normal.x); add(base + 5u, d_normal.y); add(base + 6u, d_normal.z);
            add(base + 7u, d_u.x); add(base + 8u, d_u.y); add(base + 9u, d_u.z);
            add(base + 10u, d_v.x); add(base + 11u, d_v.y); add(base + 12u, d_v.z);
            add(base + 13u, d_w.x); add(base + 14u, d_w.y); add(base + 15u, d_w.z);
            add(base + 16u, d_xy.x); add(base + 17u, d_xy.y);
            add(base + 18u, d_opacity);
        }
    }
}
