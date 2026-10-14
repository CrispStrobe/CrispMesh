//! Neural surface: a signed distance network trained by rendering the photos (NeuS-facto, written anew from the
//! configuration of `neus-facto` in hummat/sdfstudio 12a3f75, Apache-2.0, and the NeuS paper), on WebGPU.
//!
//! The model is the one of the MLX prototype `scripts/neural_surface/neus_facto_mlx.py`, and checked against it
//! number for number (`tests`): a 16-level hash grid (2 features per level) and 6 position frequencies feed a
//! geometry network (2 hidden layers, softplus with beta 100, weight normalisation) whose first output is the
//! distance and the rest geometry features; a colour network (2 hidden layers, ReLU, sigmoid) shades each sample
//! from position, view direction, normal and features. Samples are rendered with NeuS opacities.
//!
//! No automatic differentiation: every kernel has a hand-written backward kernel. The distance's gradient (needed
//! for the normals, the NeuS cosine and the eikonal term) comes from forward tangents: each sample is four rows, the
//! value and its derivatives along x, y and z. A linear layer maps all four rows alike (the bias only the value),
//! so the forward and backward passes are ordinary matrix products, and the second derivatives the eikonal term
//! needs follow from the activations' backward rule (act'' times tangent).

use anyhow::{ensure, Result};
use bytemuck::{Pod, Zeroable};

use crate::gpu::{Gpu, Kernel};

#[cfg(test)]
mod tests;
pub mod train;

/// Resolutions of the hash grid's 16 levels: floor(16 b^l) with b = (2048 / 16)^(1/15), as NumPy computes them.
pub const SCALES: [f32; 16] = [16., 22., 30., 42., 58., 80., 111., 153., 212., 294., 406., 561., 776., 1072., 1482., 2048.];
/// Width of the geometry network's input: position, 2 x 18 frequencies, 16 x 2 hash features.
pub const ENCODED: u32 = 71;
/// Rows of the weight-gradient chunks (surface_matmul mode 2).
const CHUNK: u32 = 1024;
/// Samples the sampler can hold per ray.
pub const SAMPLER_SLOTS: u32 = 128;

/// Sizes of the networks.
#[derive(Clone, Copy, Debug)]
pub struct Shape {
    pub hidden: u32,
    /// Geometry features beside the distance.
    pub features: u32,
    /// Entries per hash level, as a power of two.
    pub table_log2: u32,
}

impl Shape {
    pub fn table_size(&self) -> u32 {
        1 << self.table_log2
    }
    pub fn table_floats(&self) -> u64 {
        16 * self.table_size() as u64 * 2
    }
    /// Width of the colour network's input.
    pub fn colour_input(&self) -> u32 {
        3 + 3 + 24 + 3 + self.features
    }
}

/// One weight-normalised linear layer: offsets of v (outputs x inputs), g and b in the parameter vector, and of the
/// effective weight and bias in the weight buffer.
#[derive(Clone, Copy, Debug)]
pub struct Layer {
    pub inputs: u32,
    pub outputs: u32,
    pub v: u32,
    pub g: u32,
    pub b: u32,
    pub w: u32,
    pub wb: u32,
}

/// The six layers (geometry 0..3, colour 3..6) in MLX's parameter order, and the variance's offset.
pub fn layers(shape: &Shape) -> (Vec<Layer>, u32, u32) {
    let h = shape.hidden;
    let sizes = [(ENCODED, h), (h, h), (h, 1 + shape.features), (shape.colour_input(), h), (h, h), (h, 3)];
    let (mut parameters, mut weights) = (0, 0);
    let mut out = Vec::new();
    for (inputs, outputs) in sizes {
        let v = parameters;
        let g = v + inputs * outputs;
        let b = g + outputs;
        parameters = b + outputs;
        out.push(Layer { inputs, outputs, v, g, b, w: weights, wb: weights + inputs * outputs });
        weights += inputs * outputs + outputs;
    }
    (out, parameters, weights)
}

/// MLX's names of the parameters of `layers` (index 0..6), as in a saved checkpoint.
pub fn layer_name(index: usize) -> String {
    if index < 3 {
        format!("geometry.{index}")
    } else {
        format!("colour.{}", index - 3)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MatmulParams {
    m: u32,
    n: u32,
    k: u32,
    mode: u32,
    a_off: u32,
    b_off: u32,
    c_off: u32,
    bias_off: u32,
    bias_stride: u32,
    chunk: u32,
    pad: [u32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct EncodeParams {
    count: u32,
    tangents: u32,
    table_size: u32,
    pad: u32,
    scale: [f32; 16],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Quad {
    a: u32,
    b: u32,
    c: u32,
    d: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RenderParams {
    rays: u32,
    per_ray: u32,
    geo: u32,
    variance: u32,
    anneal: f32,
    inv_rays: f32,
    inv_samples: f32,
    eikonal: f32,
    mask: f32,
    pad: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct NormaliseParams {
    outputs: u32,
    inputs: u32,
    v: u32,
    g: u32,
    b: u32,
    w: u32,
    wb: u32,
    pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct AdamParams {
    count: u32,
    pad: u32,
    rate: f32,
    correction: f32,
    beta1: f32,
    beta2: f32,
    eps: f32,
    pad1: f32,
}

pub struct Kernels {
    /// The same kernel per mode (forward, input gradient, weight gradient), so that profiles tell them apart.
    matmul: [Kernel; 3],
    encode: Kernel,
    encode_backward: Kernel,
    activate: Kernel,
    activate_backward: Kernel,
    colour_input: Kernel,
    combine: Kernel,
    render: Kernel,
    reduce: Kernel,
    bias_grad: Kernel,
    variance_grad: Kernel,
    normalise: Kernel,
    normalise_backward: Kernel,
    adam: Kernel,
    band: Kernel,
    coarse: Kernel,
    round: Kernel,
}

/// The matrix-product kernel for one mode, its accumulation and stores written out (see the shader).
pub fn matmul_source(mode: u32) -> String {
    let mut accumulators = String::new();
    let mut inner = String::new();
    let mut store = String::new();
    for i in 0..8 {
        for h in 0..2 {
            accumulators += &format!("    var c{i}{h} = vec4<f32>(0.0);\n");
        }
    }
    for h in 0..2 {
        let at = |q: u32| format!("tile_b[base + tc + {}u]", 8 * (4 * h + q));
        inner += &format!("            let b{h} = vec4<f32>({}, {}, {}, {});\n", at(0), at(1), at(2), at(3));
    }
    for i in 0..8 {
        inner += &format!("            let a{i} = tile_a[base + tr + {}u];\n", 8 * i);
        for h in 0..2 {
            inner += &format!("            c{i}{h} += a{i} * b{h};\n");
        }
    }
    for i in 0..8 {
        for h in 0..2 {
            for q in 0..4 {
                store += &format!(
                    "    store(m0 + tr + {}u, n0 + tc + {}u, chunk, c{i}{h}.{});\n",
                    8 * i,
                    8 * (4 * h + q),
                    ["x", "y", "z", "w"][q as usize]
                );
            }
        }
    }
    let template = include_str!("../shaders/surface_matmul.wgsl");
    format!(
        "const MODE: u32 = {mode}u;\n{}",
        template.replace("    //ACCUMULATORS\n", &accumulators).replace("            //INNER\n", &inner).replace("    //STORE\n", &store)
    )
}

impl Kernels {
    pub async fn new(gpu: &Gpu) -> Result<Self> {
        Ok(Kernels {
            matmul: [
                gpu.kernel("surface_matmul_forward", &matmul_source(0), "main").await?,
                gpu.kernel("surface_matmul_input", &matmul_source(1), "main").await?,
                gpu.kernel("surface_matmul_weight", &matmul_source(2), "main").await?,
            ],
            encode: gpu.kernel("surface_encode", include_str!("../shaders/surface_encode.wgsl"), "main").await?,
            encode_backward: gpu.kernel("surface_encode_backward", include_str!("../shaders/surface_encode_backward.wgsl"), "main").await?,
            activate: gpu.kernel("surface_activate", include_str!("../shaders/surface_activate.wgsl"), "main").await?,
            activate_backward: gpu
                .kernel("surface_activate_backward", include_str!("../shaders/surface_activate_backward.wgsl"), "main")
                .await?,
            colour_input: gpu.kernel("surface_colour_input", include_str!("../shaders/surface_colour_input.wgsl"), "main").await?,
            combine: gpu.kernel("surface_combine", include_str!("../shaders/surface_combine.wgsl"), "main").await?,
            render: gpu.kernel("surface_render", include_str!("../shaders/surface_render.wgsl"), "main").await?,
            reduce: gpu.kernel("surface_reduce", include_str!("../shaders/surface_reduce.wgsl"), "main").await?,
            bias_grad: gpu.kernel("surface_bias_grad", include_str!("../shaders/surface_bias_grad.wgsl"), "main").await?,
            variance_grad: gpu.kernel("surface_variance_grad", include_str!("../shaders/surface_variance_grad.wgsl"), "main").await?,
            normalise: gpu.kernel("surface_normalise", include_str!("../shaders/surface_normalise.wgsl"), "main").await?,
            normalise_backward: gpu
                .kernel("surface_normalise_backward", include_str!("../shaders/surface_normalise_backward.wgsl"), "main")
                .await?,
            adam: gpu.kernel("surface_adam", include_str!("../shaders/surface_adam.wgsl"), "main").await?,
            band: gpu.kernel("surface_band", include_str!("../shaders/surface_band.wgsl"), "main").await?,
            coarse: gpu.kernel("surface_coarse", include_str!("../shaders/surface_coarse.wgsl"), "main").await?,
            round: gpu.kernel("surface_round", include_str!("../shaders/surface_round.wgsl"), "main").await?,
        })
    }
}

/// Trainable state: hash table and network parameters (with gradients and Adam moments), effective weights.
pub struct Model {
    pub shape: Shape,
    pub layers: Vec<Layer>,
    pub variance: u32,
    pub table: wgpu::Buffer,
    pub table_grad: wgpu::Buffer,
    table_moments: (wgpu::Buffer, wgpu::Buffer),
    pub parameters: wgpu::Buffer,
    pub parameters_grad: wgpu::Buffer,
    parameter_moments: (wgpu::Buffer, wgpu::Buffer),
    parameter_count: u32,
    pub weights: wgpu::Buffer,
    pub weights_grad: wgpu::Buffer,
}

impl Model {
    /// A model from a table (16 x 2^table_log2 x 2 floats) and the parameter vector in the order of [`layers`].
    pub fn new(gpu: &Gpu, shape: Shape, table: &[f32], parameters: &[f32]) -> Result<Self> {
        let (layers, count, weight_count) = layers(&shape);
        ensure!(table.len() as u64 == shape.table_floats(), "table has {} floats, expected {}", table.len(), shape.table_floats());
        ensure!(parameters.len() == count as usize + 1, "{} parameters, expected {}", parameters.len(), count + 1);
        let table_bytes = shape.table_floats() * 4;
        let parameter_bytes = (count as u64 + 1) * 4;
        Ok(Model {
            shape,
            layers,
            variance: count,
            table: gpu.upload("surface table", table),
            table_grad: gpu.zeroed("surface table gradient", table_bytes),
            table_moments: (gpu.zeroed("surface table m", table_bytes), gpu.zeroed("surface table v", table_bytes)),
            parameters: gpu.upload("surface parameters", parameters),
            parameters_grad: gpu.zeroed("surface parameter gradient", parameter_bytes),
            parameter_moments: (gpu.zeroed("surface parameters m", parameter_bytes), gpu.zeroed("surface parameters v", parameter_bytes)),
            parameter_count: count + 1,
            weights: gpu.zeroed("surface weights", weight_count as u64 * 4),
            weights_grad: gpu.zeroed("surface weight gradient", weight_count as u64 * 4),
        })
    }

    /// Adam starts afresh (as the prototype does after the hull fit and on resuming).
    pub fn reset_moments(&self, gpu: &Gpu) {
        for buffer in [&self.table_moments.0, &self.table_moments.1, &self.parameter_moments.0, &self.parameter_moments.1] {
            gpu.clear(buffer);
        }
    }

    /// Clears the gradients and computes the effective weights; call before the passes of a step.
    pub async fn begin_step(&self, gpu: &Gpu, kernels: &Kernels) -> Result<()> {
        gpu.clear(&self.table_grad);
        gpu.clear(&self.parameters_grad);
        gpu.clear(&self.weights_grad);
        for layer in &self.layers {
            let uniform = gpu.uniform("surface normalise", &normalise_params(layer));
            gpu.run(&kernels.normalise, &[&uniform, &self.parameters, &self.weights], layer.outputs as u64).await?;
        }
        Ok(())
    }

    /// Turns the effective weights' gradients into those of v and g; call after the passes of a step.
    pub async fn end_gradients(&self, gpu: &Gpu, kernels: &Kernels) -> Result<()> {
        for layer in &self.layers {
            let uniform = gpu.uniform("surface normalise backward", &normalise_params(layer));
            gpu.run(
                &kernels.normalise_backward,
                &[&uniform, &self.parameters, &self.weights_grad, &self.parameters_grad],
                layer.outputs as u64,
            )
            .await?;
        }
        Ok(())
    }

    /// One Adam step on every parameter (no bias correction when `corrected` is false, as MLX's default).
    pub async fn adam(&self, gpu: &Gpu, kernels: &Kernels, rate: f32, step: u32, corrected: bool) -> Result<()> {
        let (beta1, beta2) = (0.9f32, 0.999f32);
        let (rate, correction) = if corrected {
            let t = step as i32 + 1;
            (rate / (1.0 - beta1.powi(t)), 1.0 / (1.0 - beta2.powi(t)))
        } else {
            (rate, 1.0)
        };
        for (parameters, grad, (m, v), count) in [
            (&self.table, &self.table_grad, &self.table_moments, self.shape.table_floats() as u32),
            (&self.parameters, &self.parameters_grad, &self.parameter_moments, self.parameter_count),
        ] {
            let uniform = gpu.uniform("surface adam", &AdamParams { count, pad: 0, rate, correction, beta1, beta2, eps: 1e-15, pad1: 0.0 });
            gpu.run(&kernels.adam, &[&uniform, parameters, grad, m, v], count as u64).await?;
        }
        Ok(())
    }
}

fn normalise_params(layer: &Layer) -> NormaliseParams {
    NormaliseParams { outputs: layer.outputs, inputs: layer.inputs, v: layer.v, g: layer.g, b: layer.b, w: layer.w, wb: layer.wb, pad: 0 }
}

/// Buffers of one batch of rays with a fixed number of samples each.
pub struct Batch {
    pub rays: u32,
    pub per_ray: u32,
    /// Per ray: origin (3), stratified offset of the even samples, near and far (written by the sampler), unused (2).
    pub origins: wgpu::Buffer,
    /// Per ray: the sampler's sorted depths and distances ([`SAMPLER_SLOTS`] each).
    depths: wgpu::Buffer,
    values: wgpu::Buffer,
    /// Per ray: direction (3), target colour (3), mask (1), unused (1).
    pub ray_data: wgpu::Buffer,
    /// Per ray: per_ray + 1 increasing distances along the ray; sample i lies at edges[i].
    pub edges: wgpu::Buffer,
    /// Sample positions (3 per sample).
    pub points: wgpu::Buffer,
    x0: wgpu::Buffer,
    z1: wgpu::Buffer,
    h1: wgpu::Buffer,
    z2: wgpu::Buffer,
    h2: wgpu::Buffer,
    pub z3: wgpu::Buffer,
    dz3: wgpu::Buffer,
    dh2: wgpu::Buffer,
    dh1: wgpu::Buffer,
    dx0: wgpu::Buffer,
    xc: wgpu::Buffer,
    zc1: wgpu::Buffer,
    hc1: wgpu::Buffer,
    zc2: wgpu::Buffer,
    hc2: wgpu::Buffer,
    pub zc3: wgpu::Buffer,
    dzc3: wgpu::Buffer,
    dhc2: wgpu::Buffer,
    dhc1: wgpu::Buffer,
    dxc: wgpu::Buffer,
    partial: wgpu::Buffer,
    /// Per ray: L1 colour sum, eikonal sum, mask cross-entropy, sharpness gradient.
    pub losses: wgpu::Buffer,
    /// Per sample, for fitting the distance to a known field: targets and losses.
    pub targets: wgpu::Buffer,
    pub fit_losses: wgpu::Buffer,
}

impl Batch {
    pub fn new(gpu: &Gpu, shape: &Shape, rays: u32, per_ray: u32) -> Self {
        let samples = rays as u64 * per_ray as u64;
        let dual = samples * 4;
        let (h, geo, colour) = (shape.hidden as u64, 1 + shape.features as u64, shape.colour_input() as u64);
        let f = |label: &str, floats: u64| gpu.zeroed(label, floats.max(1) * 4);
        let widest = [ENCODED as u64 * h, h * h, h * geo, colour * h, h * 3].into_iter().max().unwrap_or(0);
        Batch {
            rays,
            per_ray,
            origins: f("surface origins", rays as u64 * 8),
            depths: f("surface depths", rays as u64 * SAMPLER_SLOTS as u64),
            values: f("surface values", rays as u64 * SAMPLER_SLOTS as u64),
            ray_data: f("surface rays", rays as u64 * 8),
            edges: f("surface edges", rays as u64 * (per_ray as u64 + 1)),
            points: f("surface points", samples * 3),
            x0: f("surface x0", dual * ENCODED as u64),
            z1: f("surface z1", dual * h),
            h1: f("surface h1", dual * h),
            z2: f("surface z2", dual * h),
            h2: f("surface h2", dual * h),
            z3: f("surface z3", dual * geo),
            dz3: f("surface dz3", dual * geo),
            dh2: f("surface dh2", dual * h),
            dh1: f("surface dh1", dual * h),
            dx0: f("surface dx0", dual * ENCODED as u64),
            xc: f("surface xc", samples * colour),
            zc1: f("surface zc1", samples * h),
            hc1: f("surface hc1", samples * h),
            zc2: f("surface zc2", samples * h),
            hc2: f("surface hc2", samples * h),
            zc3: f("surface zc3", samples * 3),
            dzc3: f("surface dzc3", samples * 3),
            dhc2: f("surface dhc2", samples * h),
            dhc1: f("surface dhc1", samples * h),
            dxc: f("surface dxc", samples * colour),
            partial: f("surface partial", dual.div_ceil(CHUNK as u64) * widest),
            losses: f("surface losses", rays as u64 * 4),
            targets: f("surface targets", samples),
            fit_losses: f("surface fit losses", samples),
        }
    }

    pub fn samples(&self) -> u32 {
        self.rays * self.per_ray
    }
}

/// Settings of one gradient pass.
#[derive(Clone, Copy, Debug)]
pub struct Pass {
    pub anneal: f32,
    /// Rays of the whole step (all passes): the losses are means over them.
    pub step_rays: u32,
    pub eikonal: f32,
    pub mask: f32,
}

async fn matmul(
    gpu: &Gpu,
    kernels: &Kernels,
    p: MatmulParams,
    a: &wgpu::Buffer,
    b: &wgpu::Buffer,
    c: &wgpu::Buffer,
    bias: &wgpu::Buffer,
) -> Result<()> {
    let tiles = p.m.div_ceil(64) as u64 * p.n.div_ceil(64) as u64;
    let groups = if p.mode == 2 { tiles * p.k.div_ceil(p.chunk) as u64 } else { tiles };
    let uniform = gpu.uniform("surface matmul", &p);
    gpu.run(&kernels.matmul[p.mode as usize], &[&uniform, a, b, c, bias], groups * crate::gpu::WORKGROUP as u64).await
}

fn mm(m: u32, n: u32, k: u32, mode: u32) -> MatmulParams {
    MatmulParams { m, n, k, mode, a_off: 0, b_off: 0, c_off: 0, bias_off: 0, bias_stride: 0, chunk: CHUNK, pad: [0; 2] }
}

/// z = x W^T + b for `rows` rows of a layer (bias on every `stride`-th row).
#[allow(clippy::too_many_arguments)]
async fn forward(
    gpu: &Gpu,
    kernels: &Kernels,
    model: &Model,
    layer: &Layer,
    rows: u32,
    stride: u32,
    x: &wgpu::Buffer,
    z: &wgpu::Buffer,
) -> Result<()> {
    let p = MatmulParams { b_off: layer.w, bias_off: layer.wb, bias_stride: stride, ..mm(rows, layer.outputs, layer.inputs, 0) };
    matmul(gpu, kernels, p, x, &model.weights, z, &model.weights).await
}

/// Weight and bias gradients of a layer from its output gradient dz and input x, and dx = dz W.
#[allow(clippy::too_many_arguments)]
async fn backward(
    gpu: &Gpu,
    kernels: &Kernels,
    model: &Model,
    batch: &Batch,
    layer: &Layer,
    rows: u32,
    stride: u32,
    dz: &wgpu::Buffer,
    x: &wgpu::Buffer,
    dx: Option<&wgpu::Buffer>,
) -> Result<()> {
    matmul(gpu, kernels, mm(layer.outputs, layer.inputs, rows, 2), dz, x, &batch.partial, &model.weights).await?;
    let count = layer.outputs * layer.inputs;
    let uniform = gpu.uniform("surface reduce", &Quad { a: count, b: rows.div_ceil(CHUNK), c: layer.w, d: 0 });
    gpu.run(&kernels.reduce, &[&uniform, &batch.partial, &model.weights_grad], count as u64).await?;
    let uniform = gpu.uniform("surface bias", &Quad { a: rows, b: layer.outputs, c: stride, d: layer.b });
    gpu.run(&kernels.bias_grad, &[&uniform, dz, &model.parameters_grad], layer.outputs as u64 * crate::gpu::WORKGROUP as u64).await?;
    if let Some(dx) = dx {
        let p = MatmulParams { b_off: layer.w, ..mm(rows, layer.inputs, layer.outputs, 1) };
        matmul(gpu, kernels, p, dz, &model.weights, dx, &model.weights).await?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn activate(
    gpu: &Gpu,
    kernels: &Kernels,
    count: u32,
    width: u32,
    dual: bool,
    kind: u32,
    z: &wgpu::Buffer,
    h: &wgpu::Buffer,
    back: bool,
) -> Result<()> {
    let uniform = gpu.uniform("surface activate", &Quad { a: count, b: width, c: dual as u32, d: kind });
    let kernel = if back { &kernels.activate_backward } else { &kernels.activate };
    gpu.run(kernel, &[&uniform, z, h], count as u64 * width as u64).await
}

fn encode_params(model: &Model, count: u32, tangents: bool) -> EncodeParams {
    EncodeParams { count, tangents: tangents as u32, table_size: model.shape.table_size(), pad: 0, scale: SCALES }
}

/// Geometry network with tangents for the first `count` positions of `batch.points`: distance, features and the
/// gradients in `z3` (four rows per sample).
pub async fn forward_geometry(gpu: &Gpu, kernels: &Kernels, model: &Model, batch: &Batch, count: u32) -> Result<()> {
    let r = count * 4;
    let [g0, g1, g2] = [0, 1, 2].map(|i| model.layers[i]);
    let h = model.shape.hidden;
    let uniform = gpu.uniform("surface encode", &encode_params(model, count, true));
    gpu.run(&kernels.encode, &[&uniform, &batch.points, &model.table, &batch.x0], count as u64).await?;
    forward(gpu, kernels, model, &g0, r, 4, &batch.x0, &batch.z1).await?;
    activate(gpu, kernels, count, h, true, 0, &batch.z1, &batch.h1, false).await?;
    forward(gpu, kernels, model, &g1, r, 4, &batch.h1, &batch.z2).await?;
    activate(gpu, kernels, count, h, true, 0, &batch.z2, &batch.h2, false).await?;
    forward(gpu, kernels, model, &g2, r, 4, &batch.h2, &batch.z3).await
}

/// Gradients of the geometry network and the table from the gradient of its output in `dz3` (first `count` samples).
pub async fn backward_geometry(gpu: &Gpu, kernels: &Kernels, model: &Model, batch: &Batch, count: u32) -> Result<()> {
    let r = count * 4;
    let [g0, g1, g2] = [0, 1, 2].map(|i| model.layers[i]);
    let h = model.shape.hidden;
    backward(gpu, kernels, model, batch, &g2, r, 4, &batch.dz3, &batch.h2, Some(&batch.dh2)).await?;
    activate(gpu, kernels, count, h, true, 0, &batch.z2, &batch.dh2, true).await?;
    backward(gpu, kernels, model, batch, &g1, r, 4, &batch.dh2, &batch.h1, Some(&batch.dh1)).await?;
    activate(gpu, kernels, count, h, true, 0, &batch.z1, &batch.dh1, true).await?;
    backward(gpu, kernels, model, batch, &g0, r, 4, &batch.dh1, &batch.x0, Some(&batch.dx0)).await?;
    let uniform = gpu.uniform("surface encode backward", &encode_params(model, count, true));
    gpu.run(&kernels.encode_backward, &[&uniform, &batch.points, &batch.dx0, &model.table_grad], count as u64).await
}

/// Forward pass of every sample of the batch (positions in `batch.points`): geometry with gradients in `z3`,
/// colour network outputs (before the sigmoid) in `zc3`.
pub async fn forward_batch(gpu: &Gpu, kernels: &Kernels, model: &Model, batch: &Batch) -> Result<()> {
    let s = batch.samples();
    let [c0, c1, c2] = [3, 4, 5].map(|i| model.layers[i]);
    let (h, geo) = (model.shape.hidden, 1 + model.shape.features);
    forward_geometry(gpu, kernels, model, batch, s).await?;
    let uniform = gpu.uniform("surface colour input", &Quad { a: s, b: batch.per_ray, c: geo, d: model.shape.colour_input() });
    gpu.run(&kernels.colour_input, &[&uniform, &batch.ray_data, &batch.points, &batch.z3, &batch.xc], s as u64).await?;
    forward(gpu, kernels, model, &c0, s, 1, &batch.xc, &batch.zc1).await?;
    activate(gpu, kernels, s, h, false, 1, &batch.zc1, &batch.hc1, false).await?;
    forward(gpu, kernels, model, &c1, s, 1, &batch.hc1, &batch.zc2).await?;
    activate(gpu, kernels, s, h, false, 1, &batch.zc2, &batch.hc2, false).await?;
    forward(gpu, kernels, model, &c2, s, 1, &batch.hc2, &batch.zc3).await
}

/// Renders the batch, and adds the gradients of its losses to the model's gradients.
pub async fn gradient_pass(gpu: &Gpu, kernels: &Kernels, model: &Model, batch: &Batch, pass: &Pass) -> Result<()> {
    forward_batch(gpu, kernels, model, batch).await?;
    backward_batch(gpu, kernels, model, batch, pass).await
}

/// The second half of [`gradient_pass`], after [`forward_batch`].
pub async fn backward_batch(gpu: &Gpu, kernels: &Kernels, model: &Model, batch: &Batch, pass: &Pass) -> Result<()> {
    let s = batch.samples();
    let [c0, c1, c2] = [3, 4, 5].map(|i| model.layers[i]);
    let (h, geo) = (model.shape.hidden, 1 + model.shape.features);
    let uniform = gpu.uniform(
        "surface render",
        &RenderParams {
            rays: batch.rays,
            per_ray: batch.per_ray,
            geo,
            variance: model.variance,
            anneal: pass.anneal,
            inv_rays: 1.0 / pass.step_rays as f32,
            inv_samples: 1.0 / (pass.step_rays as f32 * batch.per_ray as f32),
            eikonal: pass.eikonal,
            mask: pass.mask,
            pad: [0.0; 3],
        },
    );
    gpu.run(
        &kernels.render,
        &[&uniform, &batch.ray_data, &batch.edges, &batch.z3, &batch.zc3, &model.parameters, &batch.dz3, &batch.dzc3, &batch.losses],
        batch.rays as u64,
    )
    .await?;
    let uniform = gpu.uniform("surface variance", &Quad { a: batch.rays, b: model.variance, c: 0, d: 0 });
    gpu.run(&kernels.variance_grad, &[&uniform, &batch.losses, &model.parameters, &model.parameters_grad], 1).await?;
    // Colour network.
    backward(gpu, kernels, model, batch, &c2, s, 1, &batch.dzc3, &batch.hc2, Some(&batch.dhc2)).await?;
    activate(gpu, kernels, s, h, false, 1, &batch.zc2, &batch.dhc2, true).await?;
    backward(gpu, kernels, model, batch, &c1, s, 1, &batch.dhc2, &batch.hc1, Some(&batch.dhc1)).await?;
    activate(gpu, kernels, s, h, false, 1, &batch.zc1, &batch.dhc1, true).await?;
    backward(gpu, kernels, model, batch, &c0, s, 1, &batch.dhc1, &batch.xc, Some(&batch.dxc)).await?;
    let uniform = gpu.uniform("surface combine", &Quad { a: s, b: batch.per_ray, c: geo, d: model.shape.colour_input() });
    gpu.run(&kernels.combine, &[&uniform, &batch.dxc, &batch.z3, &batch.dz3], s as u64).await?;
    backward_geometry(gpu, kernels, model, batch, s).await
}

/// The silhouette hull's signed distance on a grid over [-1, 1]^3 (outside positive), for the sampler's band.
pub struct Hull {
    pub grid: wgpu::Buffer,
    pub resolution: u32,
}

/// The sampler's settings: even samples, then `rounds` rounds of `added` samples each at sharpness 64, 128, ...
#[derive(Clone, Copy, Debug)]
pub struct Sampling {
    pub coarse: u32,
    pub added: u32,
    pub rounds: u32,
    /// Only where the hull's distance is below this (0: the whole ray in the unit sphere).
    pub band: f32,
}

impl Sampling {
    pub fn per_ray(&self) -> u32 {
        self.coarse + self.rounds * self.added
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RoundParams {
    rays: u32,
    count: u32,
    added: u32,
    stride: u32,
    flags: u32,
    pad0: u32,
    inv_s: f32,
    pad1: f32,
}

/// Distances at the first `count` positions of `batch.points`, into `batch.z3` (one per point).
pub async fn distances(gpu: &Gpu, kernels: &Kernels, model: &Model, batch: &Batch, count: u32) -> Result<()> {
    let [g0, g1, g2] = [0, 1, 2].map(|i| model.layers[i]);
    let h = model.shape.hidden;
    let uniform = gpu.uniform("surface encode", &encode_params(model, count, false));
    gpu.run(&kernels.encode, &[&uniform, &batch.points, &model.table, &batch.x0], count as u64).await?;
    forward(gpu, kernels, model, &g0, count, 1, &batch.x0, &batch.z1).await?;
    activate(gpu, kernels, count, h, false, 0, &batch.z1, &batch.h1, false).await?;
    forward(gpu, kernels, model, &g1, count, 1, &batch.h1, &batch.z2).await?;
    activate(gpu, kernels, count, h, false, 0, &batch.z2, &batch.h2, false).await?;
    let p = MatmulParams { b_off: g2.w, bias_off: g2.wb, bias_stride: 1, ..mm(count, 1, h, 0) };
    matmul(gpu, kernels, p, &batch.h2, &model.weights, &batch.z3, &model.weights).await
}

/// The NeuS sampler for the rays in `batch.origins` and `batch.ray_data`: writes `batch.edges` and the sample
/// positions in `batch.points`. `batch.per_ray` must be `sampling.per_ray()`; the weights must be current.
pub async fn sample(gpu: &Gpu, kernels: &Kernels, model: &Model, batch: &Batch, sampling: &Sampling, hull: Option<&Hull>) -> Result<()> {
    ensure!(
        batch.per_ray == sampling.per_ray() && batch.per_ray <= SAMPLER_SLOTS,
        "batch holds {} samples per ray, sampler {}",
        batch.per_ray,
        sampling.per_ray()
    );
    ensure!(sampling.rounds >= 1 && sampling.added >= 2, "the sampler needs at least one round of two samples");
    let rays = batch.rays;
    let band = if hull.is_some() { sampling.band } else { 0.0 };
    let (grid, resolution) = match hull {
        Some(h) => (&h.grid, h.resolution),
        None => (&batch.values, 2),
    };
    let uniform = gpu.uniform("surface band", &Quad { a: rays, b: resolution, c: band.to_bits(), d: 0 });
    gpu.run(&kernels.band, &[&uniform, &batch.origins, &batch.ray_data, grid], rays as u64).await?;
    let uniform = gpu.uniform("surface coarse", &Quad { a: rays, b: sampling.coarse, c: SAMPLER_SLOTS, d: 0 });
    gpu.run(&kernels.coarse, &[&uniform, &batch.origins, &batch.ray_data, &batch.depths, &batch.points], rays as u64).await?;
    distances(gpu, kernels, model, batch, rays * sampling.coarse).await?;
    for round in 0..=sampling.rounds {
        let take = if round == 0 { 1 } else { 2 };
        let flags = take | if round == sampling.rounds { 8 } else { 4 };
        let count = if round == 0 { sampling.coarse } else { sampling.coarse + (round - 1) * sampling.added };
        let p = RoundParams {
            rays,
            count,
            added: sampling.added,
            stride: SAMPLER_SLOTS,
            flags,
            pad0: 0,
            inv_s: 64.0 * (1u32 << round.min(31)) as f32,
            pad1: 0.0,
        };
        let uniform = gpu.uniform("surface round", &p);
        gpu.run(
            &kernels.round,
            &[&uniform, &batch.origins, &batch.ray_data, &batch.z3, &batch.depths, &batch.values, &batch.points, &batch.edges],
            rays as u64,
        )
        .await?;
        if round < sampling.rounds {
            distances(gpu, kernels, model, batch, rays * sampling.added).await?;
        }
    }
    Ok(())
}
