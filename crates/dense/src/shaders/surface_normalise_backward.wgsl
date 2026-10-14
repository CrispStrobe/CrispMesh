// Gradient of v and g of one weight-normalised layer from the gradient of its effective weights W = g v / |v|.

struct Params { outputs: u32, inputs: u32, v: u32, g: u32, b: u32, w: u32, wb: u32, pad: u32 }

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> parameters: array<f32>;
@group(0) @binding(2) var<storage, read> dweights: array<f32>;
@group(0) @binding(3) var<storage, read_write> grad: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let o = id.x + id.y * ROW;
    if (o >= params.outputs) { return; }
    var squares = 0.0;
    var projection = 0.0;
    for (var i = 0u; i < params.inputs; i++) {
        let v = parameters[params.v + o * params.inputs + i];
        squares += v * v;
        projection += v * dweights[params.w + o * params.inputs + i];
    }
    let norm = sqrt(squares);
    let g = parameters[params.g + o];
    grad[params.g + o] = projection / norm;
    for (var i = 0u; i < params.inputs; i++) {
        let v = parameters[params.v + o * params.inputs + i];
        grad[params.v + o * params.inputs + i] = g / norm * (dweights[params.w + o * params.inputs + i] - projection * v / squares);
    }
}
