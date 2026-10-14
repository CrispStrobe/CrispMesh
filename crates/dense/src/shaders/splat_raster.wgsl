// Front-to-back compositing of 2D Gaussian splats per 16 x 16 tile, after gsplat's RasterizeToPixels2DGSFwd
// (Apache-2.0): ray-splat intersection in the splat's plane (or the screen-space Gaussian where that is narrower),
// alpha = min(0.999, opacity exp(-sigma)), splats below 1/255 skipped, a pixel done before transmittance would
// fall to 1e-4. One workgroup per tile; invocation t owns the pixels t, t + 64, t + 128, t + 192 of the tile (one
// column, four rows), each per-pixel quantity a vec4 over these four.
// Per pixel out (stride 16): colour sum (3), alpha, normal sum (3, camera), depth sum, distortion, median depth,
// final transmittance, last intersection + 1, intersection of the median (+ 1, 0 none), the distortion's backward
// total sum_j vis_j (d_j W_j - D_j) (W, D: opacity and depth sums up to and with j), unused (2).

struct Params { width: u32, height: u32, tiles_x: u32, pad: u32 }

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> ranges: array<u32>;
@group(0) @binding(2) var<storage, read> values: array<u32>;
@group(0) @binding(3) var<storage, read> projected: array<f32>;
@group(0) @binding(4) var<storage, read_write> pixels: array<f32>;

const STRIDE: u32 = 24u;
const OUT: u32 = 16u;

var<workgroup> batch_xy: array<vec2<f32>, 64>;
var<workgroup> batch_opacity: array<f32, 64>;
var<workgroup> batch_u: array<vec3<f32>, 64>;
var<workgroup> batch_v: array<vec3<f32>, 64>;
var<workgroup> batch_w: array<vec3<f32>, 64>;
var<workgroup> batch_colour: array<vec4<f32>, 64>;   // colour and depth
var<workgroup> batch_normal: array<vec3<f32>, 64>;
var<workgroup> finished: atomic<u32>;

fn read3(base: u32) -> vec3<f32> {
    return vec3<f32>(projected[base], projected[base + 1u], projected[base + 2u]);
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
    let end = ranges[tile * 2u + 1u];

    var T = vec4<f32>(1.0);
    var done = !inside;
    var red = vec4<f32>(0.0);
    var green = vec4<f32>(0.0);
    var blue = vec4<f32>(0.0);
    var depth_sum = vec4<f32>(0.0);
    var nx = vec4<f32>(0.0);
    var ny = vec4<f32>(0.0);
    var nz = vec4<f32>(0.0);
    var distort = vec4<f32>(0.0);
    var median = vec4<f32>(0.0);
    var median_index = vec4<u32>(0u);
    var last = vec4<u32>(0u);
    var weight_sum = vec4<f32>(0.0);
    var distort_total = vec4<f32>(0.0);

    let batches = (end - start + 63u) / 64u;
    for (var b = 0u; b < batches; b++) {
        if (t == 0u) { atomicStore(&finished, 0u); }
        workgroupBarrier();
        let first = start + b * 64u;
        let index = first + t;
        if (index < end) {
            let base = values[index] * STRIDE;
            batch_xy[t] = vec2<f32>(projected[base], projected[base + 1u]);
            batch_opacity[t] = projected[base + 18u];
            batch_u[t] = read3(base + 3u);
            batch_v[t] = read3(base + 6u);
            batch_w[t] = read3(base + 9u);
            batch_colour[t] = vec4<f32>(read3(base + 15u), projected[base + 2u]);
            batch_normal[t] = read3(base + 12u);
        }
        workgroupBarrier();
        let size = min(64u, end - first);
        for (var j = 0u; j < size; j++) {
            if (all(done)) { break; }
            let w = batch_w[j];
            let h_u = px * w - batch_u[j];
            let vx = py * w.x - batch_v[j].x;
            let vy = py * w.y - batch_v[j].y;
            let vz = py * w.z - batch_v[j].z;
            // cross(h_u, h_v) per pixel
            let cx = h_u.y * vz - h_u.z * vy;
            let cy = h_u.z * vx - h_u.x * vz;
            let cz = h_u.x * vy - h_u.y * vx;
            let nonzero = cz != vec4<f32>(0.0);
            let safe = select(vec4<f32>(1.0), cz, nonzero);
            let su = cx / safe;
            let sv = cy / safe;
            let weight_3d = su * su + sv * sv;
            let dx = batch_xy[j].x - px;
            let dy = vec4<f32>(batch_xy[j].y) - py;
            let weight_2d = 2.0 * (dx * dx + dy * dy);
            let sigma = 0.5 * min(weight_3d, weight_2d);
            let alpha = min(vec4<f32>(0.999), batch_opacity[j] * exp(-sigma));
            var use_ = !done & nonzero & (sigma >= vec4<f32>(0.0)) & (alpha >= vec4<f32>(1.0 / 255.0));
            let next_T = T * (1.0 - alpha);
            let stop = use_ & (next_T <= vec4<f32>(1e-4));
            done = done | stop;
            use_ = use_ & !stop;
            let vis = select(vec4<f32>(0.0), alpha * T, use_);
            let c = batch_colour[j];
            red += vis * c.x;
            green += vis * c.y;
            blue += vis * c.z;
            let n = batch_normal[j];
            nx += vis * n.x;
            ny += vis * n.y;
            nz += vis * n.z;
            distort += 2.0 * (vis * c.w * (1.0 - T) - vis * depth_sum);
            depth_sum += vis * c.w;
            weight_sum += vis;
            distort_total += vis * (c.w * weight_sum - depth_sum);
            let at = vec4<u32>(first + j + 1u);
            let below_half = use_ & (T > vec4<f32>(0.5));
            median = select(median, vec4<f32>(c.w), below_half);
            median_index = select(median_index, at, below_half);
            last = select(last, at, use_);
            T = select(T, next_T, use_);
        }
        atomicAdd(&finished, u32(done.x) + u32(done.y) + u32(done.z) + u32(done.w));
        if (workgroupUniformLoad(&finished) == 256u) { break; }
    }
    for (var k = 0u; k < 4u; k++) {
        if (!inside[k]) { continue; }
        let o = (ys[k] * params.width + x) * OUT;
        pixels[o] = red[k];
        pixels[o + 1u] = green[k];
        pixels[o + 2u] = blue[k];
        pixels[o + 3u] = 1.0 - T[k];
        pixels[o + 4u] = nx[k];
        pixels[o + 5u] = ny[k];
        pixels[o + 6u] = nz[k];
        pixels[o + 7u] = depth_sum[k];
        pixels[o + 8u] = distort[k];
        pixels[o + 9u] = median[k];
        pixels[o + 10u] = T[k];
        pixels[o + 11u] = bitcast<f32>(last[k]);
        pixels[o + 12u] = bitcast<f32>(median_index[k]);
        pixels[o + 13u] = distort_total[k];
    }
}
