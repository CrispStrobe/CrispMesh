// Gradients of the splats' projected values from the gradients of the rendered images, after gsplat's
// RasterizeToPixels2DGSBwd (Apache-2.0): each tile's splats back to front from the pixel's last one, transmittance
// recovered by division, running sums of what lies behind. Per pixel in (from surface losses, stride 12): colour (3),
// alpha, normal sum in camera space (3), distortion, median depth, unused (3). Per splat out (stride 20, added
// with compare-and-swap): colour (3), depth, normal (3), u, v, w rows of the ray transform (9), means2d (2),
// opacity (after the sigmoid), unused. Within a workgroup the 64 invocations' contributions to one splat are summed
// in workgroup memory (no subgroup operations), then added to memory once per splat and tile.

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
const PARTS: u32 = 19u;

var<workgroup> batch_id: array<u32, 64>;
var<workgroup> batch_xy: array<vec2<f32>, 64>;
var<workgroup> batch_opacity: array<f32, 64>;
var<workgroup> batch_u: array<vec3<f32>, 64>;
var<workgroup> batch_v: array<vec3<f32>, 64>;
var<workgroup> batch_w: array<vec3<f32>, 64>;
var<workgroup> batch_colour: array<vec4<f32>, 64>;
var<workgroup> batch_normal: array<vec3<f32>, 64>;
// contributions[invocation * PARTS + part] for the splat being processed (consecutive parts per invocation, so the
// summing invocations read consecutive words), then the batch's sums[splat * PARTS + part].
var<workgroup> contributions: array<f32, 1216>;
var<workgroup> sums: array<f32, 1216>;
var<workgroup> furthest: atomic<u32>;

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

fn component(v: vec4<f32>) -> f32 {
    return v.x + v.y + v.z + v.w;
}

@compute @workgroup_size(64)
fn main(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let tile = group.x + group.y * (ROW / 64u);
    let tiles_y = (params.height + 15u) / 16u;
    if (tile >= params.tiles_x * tiles_y) { return; }
    let tx = tile % params.tiles_x;
    let ty = tile / params.tiles_x;
    let x = tx * 16u + t % 16u;
    let ys = vec4<u32>(ty * 16u + t / 16u) + vec4<u32>(0u, 4u, 8u, 12u);
    let px = f32(x) + 0.5;
    let py = vec4<f32>(ys) + 0.5;
    let inside = vec4<bool>(x < params.width) & (ys < vec4<u32>(params.height));
    let start = ranges[tile * 2u];
    let full_end = ranges[tile * 2u + 1u];

    // Per-pixel state from the forward pass and the image gradients, as vec4 over the four pixels.
    var T_final = vec4<f32>(1.0);
    var accum_d = vec4<f32>(0.0);
    var accum_w = vec4<f32>(0.0);
    var bin_final = vec4<u32>(0u);
    var median_at = vec4<u32>(0u);
    var dr = vec4<f32>(0.0);
    var dg = vec4<f32>(0.0);
    var db = vec4<f32>(0.0);
    var da = vec4<f32>(0.0);
    var dnx = vec4<f32>(0.0);
    var dny = vec4<f32>(0.0);
    var dnz = vec4<f32>(0.0);
    var ddistort = vec4<f32>(0.0);
    var dmedian = vec4<f32>(0.0);
    for (var k = 0u; k < 4u; k++) {
        if (!inside[k]) { continue; }
        let p = ys[k] * params.width + x;
        T_final[k] = pixels[p * OUT + 10u];
        accum_d[k] = pixels[p * OUT + 7u];
        accum_w[k] = pixels[p * OUT + 3u];
        bin_final[k] = bitcast<u32>(pixels[p * OUT + 11u]);
        median_at[k] = bitcast<u32>(pixels[p * OUT + 12u]);
        dr[k] = dpixels[p * DIN];
        dg[k] = dpixels[p * DIN + 1u];
        db[k] = dpixels[p * DIN + 2u];
        da[k] = dpixels[p * DIN + 3u];
        dnx[k] = dpixels[p * DIN + 4u];
        dny[k] = dpixels[p * DIN + 5u];
        dnz[k] = dpixels[p * DIN + 6u];
        ddistort[k] = dpixels[p * DIN + 7u];
        dmedian[k] = dpixels[p * DIN + 8u];
    }
    // Splats behind every pixel's last one contribute nothing: start at the furthest last one of the tile.
    if (t == 0u) { atomicStore(&furthest, 0u); }
    workgroupBarrier();
    atomicMax(&furthest, max(max(bin_final.x, bin_final.y), max(bin_final.z, bin_final.w)));
    let end = min(full_end, workgroupUniformLoad(&furthest));
    var T = T_final;
    var buffer_r = vec4<f32>(0.0);
    var buffer_g = vec4<f32>(0.0);
    var buffer_b = vec4<f32>(0.0);
    var buffer_nx = vec4<f32>(0.0);
    var buffer_ny = vec4<f32>(0.0);
    var buffer_nz = vec4<f32>(0.0);
    var accum_d_buffer = accum_d;
    var accum_w_buffer = accum_w;
    var distort_buffer = vec4<f32>(0.0);

    var batches = 0u;
    if (end > start) { batches = (end - start + 63u) / 64u; }
    for (var b = 0u; b < batches; b++) {
        workgroupBarrier();
        // Batch b holds the splats [last - 63, last] of what is left, the furthest at slot 0.
        let last = end - 1u - b * 64u;
        let size = min(64u, last + 1u - start);
        if (t < size) {
            let index = last - t;
            let g = values[index];
            let base = g * STRIDE;
            batch_id[t] = g;
            batch_xy[t] = vec2<f32>(projected[base], projected[base + 1u]);
            batch_opacity[t] = projected[base + 18u];
            batch_u[t] = read3(base + 3u);
            batch_v[t] = read3(base + 6u);
            batch_w[t] = read3(base + 9u);
            batch_colour[t] = vec4<f32>(read3(base + 15u), projected[base + 2u]);
            batch_normal[t] = read3(base + 12u);
        }
        for (var c = 0u; c < PARTS; c++) { sums[c * 64u + t] = 0.0; }
        workgroupBarrier();
        for (var j = 0u; j < size; j++) {
            let index = last - j;               // intersection index of this splat
            let at = vec4<u32>(index + 1u);
            var valid = inside & (at <= bin_final);
            let opacity = batch_opacity[j];
            let u = batch_u[j];
            let v = batch_v[j];
            let w = batch_w[j];
            let h_u = px * w - u;
            let vx = py * w.x - v.x;
            let vy = py * w.y - v.y;
            let vz = py * w.z - v.z;
            let cx = h_u.y * vz - h_u.z * vy;
            let cy = h_u.z * vx - h_u.x * vz;
            let cz = h_u.x * vy - h_u.y * vx;
            valid = valid & (cz != vec4<f32>(0.0));
            let safe = select(vec4<f32>(1.0), cz, cz != vec4<f32>(0.0));
            let sx = cx / safe;
            let sy = cy / safe;
            let weight_3d = sx * sx + sy * sy;
            let dx = batch_xy[j].x - px;
            let dy = vec4<f32>(batch_xy[j].y) - py;
            let weight_2d = 2.0 * (dx * dx + dy * dy);
            let sigma = 0.5 * min(weight_3d, weight_2d);
            let vis = exp(-sigma);
            let alpha = min(vec4<f32>(0.999), opacity * vis);
            valid = valid & (sigma >= vec4<f32>(0.0)) & (alpha >= vec4<f32>(1.0 / 255.0));

            let colour = batch_colour[j];
            let normal = batch_normal[j];
            let ra = 1.0 / (1.0 - alpha);
            let T_here = select(T, T * ra, valid);
            let fac = select(vec4<f32>(0.0), alpha * T_here, valid);
            // Colour, depth and normal gradients.
            var d_depth = select(vec4<f32>(0.0), dmedian, valid & (median_at == at));
            var v_alpha = (colour.x * T_here - buffer_r * ra) * dr + (colour.y * T_here - buffer_g * ra) * dg + (colour.z * T_here - buffer_b * ra) * db;
            v_alpha += (normal.x * T_here - buffer_nx * ra) * dnx + (normal.y * T_here - buffer_ny * ra) * dny + (normal.z * T_here - buffer_nz * ra) * dnz;
            v_alpha += T_final * ra * da;
            let dl_dw = 2.0 * (2.0 * (colour.w * accum_w_buffer - accum_d_buffer) + (accum_d - colour.w * accum_w));
            v_alpha += (dl_dw * T_here - distort_buffer * ra) * ddistort;
            d_depth += select(vec4<f32>(0.0), 2.0 * fac * (2.0 - 2.0 * T_here - accum_w + fac) * ddistort, valid);
            v_alpha = select(vec4<f32>(0.0), v_alpha, valid);
            // Geometry and opacity, unless alpha was clipped at 0.999.
            let unclipped = valid & (opacity * vis <= vec4<f32>(0.999));
            let v_G = select(vec4<f32>(0.0), opacity * v_alpha, unclipped);
            let use_3d = unclipped & (weight_3d <= weight_2d);
            let use_2d = unclipped & !(weight_3d <= weight_2d);
            // Case 1: the ray-splat intersection.
            let vsx = select(vec4<f32>(0.0), v_G * -vis * sx / safe, use_3d);
            let vsy = select(vec4<f32>(0.0), v_G * -vis * sy / safe, use_3d);
            let vcz = -(vsx * sx + vsy * sy);
            // v_h_u = cross(h_v, v_cross), v_h_v = cross(v_cross, h_u), with v_cross = (vsx, vsy, vcz).
            let hux = vy * vcz - vz * vsy;
            let huy = vz * vsx - vx * vcz;
            let huz = vx * vsy - vy * vsx;
            let hvx = vsy * h_u.z - vcz * h_u.y;
            let hvy = vcz * h_u.x - vsx * h_u.z;
            let hvz = vsx * h_u.y - vsy * h_u.x;
            // Case 2: the screen-space Gaussian.
            let v_x = select(vec4<f32>(0.0), v_G * -vis * 2.0 * dx, use_2d);
            let v_y = select(vec4<f32>(0.0), v_G * -vis * 2.0 * dy, use_2d);
            let v_opacity = select(vec4<f32>(0.0), vis * v_alpha, unclipped);

            contributions[t * PARTS + 0u] = component(fac * dr);
            contributions[t * PARTS + 1u] = component(fac * dg);
            contributions[t * PARTS + 2u] = component(fac * db);
            contributions[t * PARTS + 3u] = component(d_depth);
            contributions[t * PARTS + 4u] = component(fac * dnx);
            contributions[t * PARTS + 5u] = component(fac * dny);
            contributions[t * PARTS + 6u] = component(fac * dnz);
            contributions[t * PARTS + 7u] = component(-hux);
            contributions[t * PARTS + 8u] = component(-huy);
            contributions[t * PARTS + 9u] = component(-huz);
            contributions[t * PARTS + 10u] = component(-hvx);
            contributions[t * PARTS + 11u] = component(-hvy);
            contributions[t * PARTS + 12u] = component(-hvz);
            contributions[t * PARTS + 13u] = component(px * hux + py * hvx);
            contributions[t * PARTS + 14u] = component(px * huy + py * hvy);
            contributions[t * PARTS + 15u] = component(px * huz + py * hvz);
            contributions[t * PARTS + 16u] = component(v_x);
            contributions[t * PARTS + 17u] = component(v_y);
            contributions[t * PARTS + 18u] = component(v_opacity);

            // What lies behind, for the splats in front.
            buffer_r += colour.x * fac;
            buffer_g += colour.y * fac;
            buffer_b += colour.z * fac;
            buffer_nx += normal.x * fac;
            buffer_ny += normal.y * fac;
            buffer_nz += normal.z * fac;
            accum_d_buffer -= select(vec4<f32>(0.0), fac * colour.w, valid);
            accum_w_buffer -= fac;
            distort_buffer += select(vec4<f32>(0.0), dl_dw * fac, valid);
            T = T_here;
            workgroupBarrier();
            if (t < PARTS) {
                var total = 0.0;
                for (var k = 0u; k < 64u; k++) { total += contributions[k * PARTS + t]; }
                sums[j * PARTS + t] = total;
            }
            workgroupBarrier();
        }
        if (t < size) {
            let base = batch_id[t] * G;
            for (var c = 0u; c < PARTS; c++) { add(base + c, sums[t * PARTS + c]); }
        }
    }
}
