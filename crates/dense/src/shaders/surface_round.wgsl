// One round of the NeuS sampler per ray, on the sorted depths and distances kept per ray (`stride` slots).
// flags 1: take the `count` even samples' distances from `evaluated`. 2: merge the `added` samples written after the
// first `count` (their distances in `evaluated`), keeping the order. 4: draw `added` new samples where the opacities
// at sharpness `inv_s` put weight (deterministic quantiles), after the kept ones, with their positions for
// evaluation. 8: write the `count` samples and the far end as the rendering edges, with their positions.

struct Params { rays: u32, count: u32, added: u32, stride: u32, flags: u32, pad0: u32, inv_s: f32, pad1: f32 }

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> origins: array<f32>;
@group(0) @binding(2) var<storage, read> rays: array<f32>;
@group(0) @binding(3) var<storage, read> evaluated: array<f32>;
@group(0) @binding(4) var<storage, read_write> depths: array<f32>;
@group(0) @binding(5) var<storage, read_write> values: array<f32>;
@group(0) @binding(6) var<storage, read_write> points: array<f32>;
@group(0) @binding(7) var<storage, read_write> edges: array<f32>;

const MAX: u32 = 129u;

fn sigmoid(v: f32) -> f32 {
    return 1.0 / (1.0 + exp(-v));
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let ray = id.x + id.y * ROW;
    if (ray >= params.rays) { return; }
    let o = vec3<f32>(origins[ray * 8u], origins[ray * 8u + 1u], origins[ray * 8u + 2u]);
    let d = vec3<f32>(rays[ray * 8u], rays[ray * 8u + 1u], rays[ray * 8u + 2u]);
    let base = ray * params.stride;
    var n = params.count;
    if ((params.flags & 1u) != 0u) {
        for (var i = 0u; i < n; i++) { values[base + i] = evaluated[ray * n + i]; }
    }
    if ((params.flags & 2u) != 0u) {
        // Insertion of each added sample into the sorted list.
        for (var j = 0u; j < params.added; j++) {
            let z = depths[base + n + j];
            let v = evaluated[ray * params.added + j];
            var k = n + j;
            while (k > 0u && depths[base + k - 1u] > z) {
                depths[base + k] = depths[base + k - 1u];
                values[base + k] = values[base + k - 1u];
                k -= 1u;
            }
            depths[base + k] = z;
            values[base + k] = v;
        }
        n += params.added;
    }
    if ((params.flags & 4u) != 0u) {
        // cdf[i + 1] = cdf[i] + pdf[i], pdf = (weight + 1e-5) / sum, weights from the NeuS opacities.
        var cdf: array<f32, MAX>;
        var total = 0.0;
        var through = 1.0;
        var previous_cos = 0.0;
        let intervals = n - 1u;
        cdf[0] = 0.0;
        for (var i = 0u; i < intervals; i++) {
            let z0 = depths[base + i];
            let z1 = depths[base + i + 1u];
            let v0 = values[base + i];
            let v1 = values[base + i + 1u];
            let mid = 0.5 * (v0 + v1);
            let dist = z1 - z0;
            let raw_cos = (v1 - v0) / (dist + 1e-5);
            let cos = clamp(min(previous_cos, raw_cos), -1e3, 0.0);
            previous_cos = raw_cos;
            let previous = sigmoid((mid - cos * dist * 0.5) * params.inv_s);
            let following = sigmoid((mid + cos * dist * 0.5) * params.inv_s);
            let alpha = (previous - following + 1e-5) / (previous + 1e-5);
            let w = alpha * through + 1e-5;
            through *= 1.0 - alpha + 1e-7;
            cdf[i + 1u] = w;
            total += w;
        }
        for (var i = 0u; i < intervals; i++) {
            cdf[i + 1u] = cdf[i] + cdf[i + 1u] / total;
        }
        let first = 0.5 / f32(params.added);
        let spacing = (1.0 - 0.5 / f32(params.added) - first) / f32(params.added - 1u);
        for (var j = 0u; j < params.added; j++) {
            let u = first + f32(j) * spacing;
            var below = 0;
            for (var k = 0u; k <= intervals; k++) {
                if (u >= cdf[k]) { below += 1; }
            }
            let index = u32(clamp(below - 1, 0, i32(intervals) - 1));
            let c0 = cdf[index];
            let c1 = cdf[index + 1u];
            let z0 = depths[base + index];
            let z1 = depths[base + index + 1u];
            let z = z0 + (z1 - z0) * clamp((u - c0) / max(c1 - c0, 1e-8), 0.0, 1.0);
            depths[base + n + j] = z;
            let p = o + z * d;
            let s = ray * params.added + j;
            points[s * 3u] = p.x;
            points[s * 3u + 1u] = p.y;
            points[s * 3u + 2u] = p.z;
        }
    }
    if ((params.flags & 8u) != 0u) {
        for (var i = 0u; i < n; i++) {
            let z = depths[base + i];
            edges[ray * (n + 1u) + i] = z;
            let p = o + z * d;
            let s = ray * n + i;
            points[s * 3u] = p.x;
            points[s * 3u + 1u] = p.y;
            points[s * 3u + 2u] = p.z;
        }
        edges[ray * (n + 1u) + n] = origins[ray * 8u + 5u];
    }
}
