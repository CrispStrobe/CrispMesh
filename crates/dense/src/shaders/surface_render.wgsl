// Volume rendering of one ray (NeuS opacities from the distance and its gradient, colours from the colour network),
// its losses and their gradients per sample, in one pass. Losses: L1 colour (mean over rays and channels), eikonal
// (mean over all samples), mask cross-entropy with the opacity clipped to [1e-3, 1 - 1e-3] (mean over rays).
// Writes the gradient of the distance and of its gradient into column 0 of the geometry output's four rows, the
// gradient of the colour network's output, and per ray: L1 sum, eikonal sum, cross-entropy, sharpness gradient.

struct Params {
    rays: u32, per_ray: u32, geo: u32, variance: u32,
    anneal: f32, inv_rays: f32, inv_samples: f32, eikonal: f32,
    mask: f32, pad0: f32, pad1: f32, pad2: f32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> rays: array<f32>;
@group(0) @binding(2) var<storage, read> edges: array<f32>;
@group(0) @binding(3) var<storage, read> geometry: array<f32>;
@group(0) @binding(4) var<storage, read> colour: array<f32>;
@group(0) @binding(5) var<storage, read> parameters: array<f32>;
@group(0) @binding(6) var<storage, read_write> dgeometry: array<f32>;
@group(0) @binding(7) var<storage, read_write> dcolour: array<f32>;
@group(0) @binding(8) var<storage, read_write> losses: array<f32>;

const MAX_SAMPLES: u32 = 128u;

fn sigmoid(v: f32) -> f32 {
    return 1.0 / (1.0 + exp(-v));
}

struct Interval { alpha: f32, raw: f32, previous: f32, following: f32, half: f32, cosine: f32 }

fn interval(distance: f32, cosine: f32, delta: f32, sharpness: f32) -> Interval {
    let annealed = -(max(-cosine * 0.5 + 0.5, 0.0) * (1.0 - params.anneal) + max(-cosine, 0.0) * params.anneal);
    let half = annealed * delta * 0.5;
    let previous = sigmoid((distance - half) * sharpness);
    let following = sigmoid((distance + half) * sharpness);
    let raw = (previous - following + 1e-5) / (previous + 1e-5);
    return Interval(clamp(raw, 0.0, 1.0), raw, previous, following, half, cosine);
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let ray = id.x + id.y * ROW;
    if (ray >= params.rays) { return; }
    let n = params.per_ray;
    let d = vec3<f32>(rays[ray * 8u], rays[ray * 8u + 1u], rays[ray * 8u + 2u]);
    let target_colour = vec3<f32>(rays[ray * 8u + 3u], rays[ray * 8u + 4u], rays[ray * 8u + 5u]);
    let mask = rays[ray * 8u + 6u];
    let sharpness = clamp(exp(parameters[params.variance] * 10.0), 1e-6, 1e6);
    var alpha: array<f32, MAX_SAMPLES>;
    var transmittance: array<f32, MAX_SAMPLES>;
    var rendered = vec3<f32>(0.0);
    var opacity = 0.0;
    var through = 1.0;
    var eikonal = 0.0;
    for (var i = 0u; i < n; i++) {
        let s = ray * n + i;
        let row = s * 4u * params.geo;
        let g = vec3<f32>(geometry[row + params.geo], geometry[row + 2u * params.geo], geometry[row + 3u * params.geo]);
        let delta = edges[ray * (n + 1u) + i + 1u] - edges[ray * (n + 1u) + i];
        let a = interval(geometry[row], dot(d, g), delta, sharpness).alpha;
        alpha[i] = a;
        transmittance[i] = through;
        let weight = a * through;
        let rgb = vec3<f32>(sigmoid(colour[s * 3u]), sigmoid(colour[s * 3u + 1u]), sigmoid(colour[s * 3u + 2u])) * 1.002 - 0.001;
        rendered += weight * rgb;
        opacity += weight;
        through *= 1.0 - a + 1e-7;
        let norm = sqrt(dot(g, g) + 1e-12);
        eikonal += (norm - 1.0) * (norm - 1.0);
    }
    let difference = rendered - target_colour;
    let dcolour_ray = sign(difference) * (params.inv_rays / 3.0);
    let clipped = clamp(opacity, 1e-3, 1.0 - 1e-3);
    let entropy = -(mask * log(clipped) + (1.0 - mask) * log(1.0 - clipped));
    var dopacity = 0.0;
    if (opacity >= 1e-3 && opacity <= 1.0 - 1e-3) {
        dopacity = -(mask / clipped - (1.0 - mask) / (1.0 - clipped)) * params.inv_rays * params.mask;
    }
    // Backwards: `later` is the sum over later samples j of dweight_j alpha_j prod_{i<k<j} (1 - alpha_k + 1e-7).
    var later = 0.0;
    var dsharpness = 0.0;
    for (var step = 0u; step < n; step++) {
        let i = n - 1u - step;
        let s = ray * n + i;
        let row = s * 4u * params.geo;
        let g = vec3<f32>(geometry[row + params.geo], geometry[row + 2u * params.geo], geometry[row + 3u * params.geo]);
        let distance = geometry[row];
        let delta = edges[ray * (n + 1u) + i + 1u] - edges[ray * (n + 1u) + i];
        let q = interval(distance, dot(d, g), delta, sharpness);
        let sig = vec3<f32>(sigmoid(colour[s * 3u]), sigmoid(colour[s * 3u + 1u]), sigmoid(colour[s * 3u + 2u]));
        let rgb = sig * 1.002 - 0.001;
        let weight = alpha[i] * transmittance[i];
        let drgb = dcolour_ray * weight;
        let dz = drgb * 1.002 * sig * (1.0 - sig);
        dcolour[s * 3u] = dz.x;
        dcolour[s * 3u + 1u] = dz.y;
        dcolour[s * 3u + 2u] = dz.z;
        let dweight = dot(dcolour_ray, rgb) + dopacity;
        var dalpha = transmittance[i] * (dweight - later);
        later = dweight * alpha[i] + (1.0 - alpha[i] + 1e-7) * later;
        if (q.raw < 0.0 || q.raw > 1.0) { dalpha = 0.0; }
        let b = q.previous + 1e-5;
        let dprevious = dalpha * q.following / (b * b);
        let dfollowing = -dalpha / b;
        let du = dprevious * q.previous * (1.0 - q.previous);
        let dv = dfollowing * q.following * (1.0 - q.following);
        let ddistance = (du + dv) * sharpness;
        let dhalf = (dv - du) * sharpness;
        dsharpness += du * (distance - q.half) + dv * (distance + q.half);
        let dannealed = dhalf * delta * 0.5;
        var slope = 0.0;
        if (-q.cosine * 0.5 + 0.5 > 0.0) { slope += 0.5 * (1.0 - params.anneal); }
        if (-q.cosine > 0.0) { slope += params.anneal; }
        let dcosine = dannealed * slope;
        let norm = sqrt(dot(g, g) + 1e-12);
        let dg = d * dcosine + g * (params.eikonal * params.inv_samples * 2.0 * (norm - 1.0) / norm);
        dgeometry[row] = ddistance;
        dgeometry[row + params.geo] = dg.x;
        dgeometry[row + 2u * params.geo] = dg.y;
        dgeometry[row + 3u * params.geo] = dg.z;
    }
    losses[ray * 4u] = abs(difference.x) + abs(difference.y) + abs(difference.z);
    losses[ray * 4u + 1u] = eikonal;
    losses[ray * 4u + 2u] = entropy;
    losses[ray * 4u + 3u] = dsharpness;
}
