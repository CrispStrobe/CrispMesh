// Weight normalisation of one layer: W[o, :] = g[o] v[o, :] / |v[o, :]|, bias copied, into the effective weights.

struct Params { outputs: u32, inputs: u32, v: u32, g: u32, b: u32, w: u32, wb: u32, pad: u32 }

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> parameters: array<f32>;
@group(0) @binding(2) var<storage, read_write> weights: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let o = id.x + id.y * ROW;
    if (o >= params.outputs) { return; }
    var squares = 0.0;
    for (var i = 0u; i < params.inputs; i++) {
        let v = parameters[params.v + o * params.inputs + i];
        squares += v * v;
    }
    let scale = parameters[params.g + o] / sqrt(squares);
    for (var i = 0u; i < params.inputs; i++) {
        weights[params.w + o * params.inputs + i] = parameters[params.v + o * params.inputs + i] * scale;
    }
    weights[params.wb + o] = parameters[params.b + o];
}
