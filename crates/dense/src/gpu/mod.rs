//! `wgpu` device handling and a small kernel-dispatch layer.
//!
//! Everything here is plain WebGPU: WGSL compute shaders, storage and uniform
//! buffers, no backend-specific calls. The device is requested with the default
//! WebGPU limits, so validation fails here whenever a kernel would not run in a
//! browser: at most 8 storage buffers per stage, 128 MiB per storage binding,
//! 256 MiB per buffer, 65535 workgroups per dimension. Callers size their work
//! from [`Gpu::binding_budget`] instead of assuming a large adapter.
//!
//! Device creation, kernel compilation, dispatch and readback are `async`: a
//! browser resolves them through its event loop and nothing may block there.
//! Native callers drive the same code with [`block_on`]; the readback then
//! polls the device itself, so the futures are ready when first polled.

use std::borrow::Cow;

use anyhow::{anyhow, Context};
use bytemuck::Pod;
use wgpu::util::DeviceExt;

/// Invocations per workgroup of every kernel (`@workgroup_size(64)`).
pub const WORKGROUP: u32 = 64;

/// Workgroups along x before a dispatch spills into y. The kernels rebuild the
/// linear index as `id.x + id.y * ROW`.
pub const ROW_GROUPS: u32 = 16384;
pub const ROW: u32 = ROW_GROUPS * WORKGROUP;

/// Commands recorded per submit in a browser (see `Gpu::pending`). `CRISP3DS_GPU_BATCH` is not
/// read: a browser has no environment; change it here to measure.
const BATCH: usize = usize::MAX;
/// Commands per submit with native batching: enough to amortise a submit, few enough to keep the GPU busy.
const NATIVE_BATCH: usize = 32;

pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub info: wgpu::AdapterInfo,
    pub limits: wgpu::Limits,
    peak: std::sync::atomic::AtomicU64,
    /// First error the device reported outside an error scope.
    uncaptured: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    /// In a browser, dispatches are recorded into one command encoder and submitted
    /// [`BATCH`] at a time, and before every readback or write: every submit is a round trip to
    /// the GPU process there (one per dispatch made 12 700 of them in the Bunny's stereo stage),
    /// while holding everything until the readback leaves the GPU idle as the work is recorded.
    /// Natively each dispatch is submitted at once, inside its error scope, as before.
    pending: std::sync::Mutex<(Option<wgpu::CommandEncoder>, usize)>,
    /// Natively: record into `pending` as in a browser ([`Gpu::set_batching`]).
    batching: std::sync::atomic::AtomicBool,
    /// Natively with `CRISP3DS_GPU_PROFILE=1`: seconds and dispatches per kernel (each dispatch waited for).
    profile: Option<std::sync::Mutex<std::collections::BTreeMap<String, (f64, u64)>>>,
    /// Storage buffers made since [`Gpu::track`], for [`Gpu::release_tracked`].
    tracked: std::sync::Mutex<Option<Vec<wgpu::Buffer>>>,
}

impl Gpu {
    /// The system's preferred adapter with default WebGPU limits (or the adapter's, where lower).
    /// `CRISP3DS_GPU_FALLBACK=1` asks for a software adapter instead (for CI).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new() -> anyhow::Result<Self> {
        block_on(Self::request())
    }

    /// [`Gpu::new`] without blocking; the only way to get a device in a browser.
    pub async fn request() -> anyhow::Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let fallback = std::env::var("CRISP3DS_GPU_FALLBACK").map(|v| v == "1").unwrap_or(false);
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: fallback,
                compatible_surface: None,
                ..Default::default()
            })
            .await
            .map_err(|e| anyhow!("no GPU adapter: {e}"))?;
        // Default WebGPU limits, lowered where the adapter offers less (the iOS simulator's
        // Metal adapter has 15 inter-stage variables, not 16). Never more than the defaults,
        // so results do not depend on how generous the GPU is.
        let limits = wgpu::Limits::default().or_worse_values_from(&adapter.limits());
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("crisp3ds-dense"),
                required_features: wgpu::Features::empty(),
                required_limits: limits.clone(),
                ..Default::default()
            })
            .await
            .context("GPU device with default WebGPU limits")?;
        let uncaptured = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
        let sink = uncaptured.clone();
        device.on_uncaptured_error(std::sync::Arc::new(move |error| {
            sink.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get_or_insert_with(|| error.to_string());
        }));
        Ok(Gpu {
            device,
            queue,
            info: adapter.get_info(),
            limits,
            peak: std::sync::atomic::AtomicU64::new(0),
            uncaptured,
            pending: std::sync::Mutex::new((None, 0)),
            batching: std::sync::atomic::AtomicBool::new(false),
            profile: (!cfg!(target_arch = "wasm32") && std::env::var("CRISP3DS_GPU_PROFILE").is_ok_and(|v| v == "1"))
                .then(|| std::sync::Mutex::new(Default::default())),
            tracked: std::sync::Mutex::new(None),
        })
    }

    /// From here on the storage buffers this device makes are remembered, until [`Gpu::release_tracked`].
    /// Only in a browser: there a dropped buffer keeps its memory until the garbage collector runs,
    /// which GPU memory does not trigger, so remembering costs nothing. Natively a dropped buffer is
    /// freed at once, and remembering would keep every buffer of the pass alive.
    pub fn track(&self) {
        if cfg!(target_arch = "wasm32") {
            *self.tracked.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Vec::new());
        }
    }

    /// Frees the memory of every storage buffer made since [`Gpu::track`]. For a pass whose results
    /// have all been read back and whose buffers are not used again: a second pass would otherwise
    /// allocate next to the first one's buffers in a browser.
    pub fn release_tracked(&self) {
        self.flush();
        if let Some(buffers) = self.tracked.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take() {
            for buffer in buffers {
                buffer.destroy();
            }
        }
    }

    fn remember(&self, buffer: wgpu::Buffer) -> wgpu::Buffer {
        if let Some(buffers) = self.tracked.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).as_mut() {
            buffers.push(buffer.clone());
        }
        buffer
    }

    pub fn describe(&self) -> String {
        format!("{} ({:?})", self.info.name, self.info.backend)
    }

    /// Largest buffer bound to a kernel so far, in bytes (128 MiB is the WebGPU default limit).
    pub fn peak_binding(&self) -> u64 {
        self.peak.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Bytes one storage binding may hold, with headroom below the device limit.
    pub fn binding_budget(&self) -> u64 {
        let limit = self.limits.max_storage_buffer_binding_size.min(self.limits.max_buffer_size);
        limit - limit / 8
    }

    /// Read-only (to kernels) storage buffer with the given content.
    pub fn upload<T: Pod>(&self, label: &str, data: &[T]) -> wgpu::Buffer {
        let bytes: &[u8] = bytemuck::cast_slice(data);
        if bytes.is_empty() {
            // Zero-sized bindings are invalid; keep one word.
            return self.zeroed(label, 4);
        }
        self.remember(self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
        }))
    }

    /// Zero-initialised storage buffer of `bytes` bytes (rounded up to 4).
    pub fn zeroed(&self, label: &str, bytes: u64) -> wgpu::Buffer {
        self.remember(self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: bytes.max(4).div_ceil(4) * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }))
    }

    pub fn uniform<T: Pod>(&self, label: &str, value: &T) -> wgpu::Buffer {
        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::bytes_of(value),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    }

    /// A queue write takes effect before the next submit, so recorded dispatches go first.
    pub fn write<T: Pod>(&self, buffer: &wgpu::Buffer, data: &[T]) {
        self.flush();
        self.queue.write_buffer(buffer, 0, bytemuck::cast_slice(data));
    }

    pub fn clear(&self, buffer: &wgpu::Buffer) {
        self.record(|encoder| encoder.clear_buffer(buffer, 0, None));
    }

    /// Natively, dispatches are submitted one by one by default. With batching on they are recorded into one
    /// encoder and submitted [`NATIVE_BATCH`] at a time and before every write and readback, as in a browser: for
    /// many small dispatches (the neural surface's ~400 per step) the per-submit cost dominates otherwise.
    /// Validation errors of the recorded commands then surface at the next readback, not at the dispatch.
    pub fn set_batching(&self, on: bool) {
        self.flush();
        self.batching.store(on, std::sync::atomic::Ordering::Relaxed);
    }

    fn batched(&self) -> bool {
        cfg!(target_arch = "wasm32") || self.batching.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Records into the pending encoder (browser, or native batching) or submits at once (native).
    fn record(&self, commands: impl FnOnce(&mut wgpu::CommandEncoder)) {
        if self.batched() {
            let mut pending = self.pending.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            commands(pending.0.get_or_insert_with(|| self.device.create_command_encoder(&Default::default())));
            pending.1 += 1;
            let limit = if cfg!(target_arch = "wasm32") { BATCH } else { NATIVE_BATCH };
            if pending.1 >= limit {
                pending.1 = 0;
                if let Some(encoder) = pending.0.take() {
                    self.queue.submit([encoder.finish()]);
                }
            }
        } else {
            let mut encoder = self.device.create_command_encoder(&Default::default());
            commands(&mut encoder);
            self.queue.submit([encoder.finish()]);
        }
    }

    /// Submits what was recorded (nothing to do without batching).
    pub fn flush(&self) {
        let mut pending = self.pending.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        pending.1 = 0;
        if let Some(encoder) = pending.0.take() {
            self.queue.submit([encoder.finish()]);
        }
    }

    /// Copies the first `count` elements of a storage buffer back to the CPU.
    pub async fn read<T: Pod>(&self, buffer: &wgpu::Buffer, count: usize) -> anyhow::Result<Vec<T>> {
        let pending = self.begin_read::<T>(buffer, count);
        self.finish_read(pending).await
    }

    /// The first half of [`Gpu::read`]: the copy to a staging buffer is queued now, behind the work
    /// already recorded, and submitted; [`Gpu::finish_read`] waits for that copy alone. Work queued
    /// in between runs on the GPU while the caller does other things, and does not delay the read.
    pub fn begin_read<T: Pod>(&self, buffer: &wgpu::Buffer, count: usize) -> PendingRead<T> {
        let bytes = (count * std::mem::size_of::<T>()) as u64;
        let padded = (bytes.div_ceil(4) * 4).max(4);
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: padded,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        #[cfg(target_arch = "wasm32")]
        let index = {
            if bytes > 0 {
                self.record(|encoder| encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, padded));
            }
            self.flush();
        };
        #[cfg(not(target_arch = "wasm32"))]
        let index = {
            self.flush();
            let mut encoder = self.device.create_command_encoder(&Default::default());
            if bytes > 0 {
                encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, padded);
            }
            self.queue.submit([encoder.finish()])
        };
        let mapped = Mapped::default();
        let signal = mapped.clone();
        staging.slice(..).map_async(wgpu::MapMode::Read, move |result| signal.complete(result));
        PendingRead { staging, mapped, count, bytes, index, element: std::marker::PhantomData }
    }

    /// The second half of [`Gpu::read`].
    pub async fn finish_read<T: Pod>(&self, pending: PendingRead<T>) -> anyhow::Result<Vec<T>> {
        let PendingRead { staging, mapped, count, bytes, index, .. } = pending;
        // A native device completes the mapping while it is polled (here only up to the copy's own
        // submission); a browser completes it from its event loop.
        #[cfg(not(target_arch = "wasm32"))]
        self.device.poll(wgpu::PollType::Wait { submission_index: Some(index), timeout: None }).map_err(|e| anyhow!("GPU poll: {e}"))?;
        #[cfg(target_arch = "wasm32")]
        let () = index;
        mapped.await.map_err(|e| anyhow!("GPU readback: {e}"))?;
        if let Some(error) = self.uncaptured.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take() {
            return Err(anyhow!("GPU: {error}"));
        }
        let mut out = vec![T::zeroed(); count];
        if bytes > 0 {
            let view = staging.slice(..).get_mapped_range().map_err(|e| anyhow!("GPU readback: {e}"))?;
            bytemuck::cast_slice_mut::<T, u8>(&mut out).copy_from_slice(&view[..bytes as usize]);
        }
        staging.unmap();
        Ok(out)
    }

    /// Compiles one compute entry point. Validation errors become `Err`.
    pub async fn kernel(&self, label: &str, source: &str, entry: &str) -> anyhow::Result<Kernel> {
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(Cow::Owned(format!("const ROW: u32 = {ROW}u;\n{source}"))),
        });
        let pipeline = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(label),
            layout: None,
            module: &module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            cache: None,
        });
        if let Some(error) = scope.pop().await {
            return Err(anyhow!("kernel {label}: {error}"));
        }
        Ok(Kernel { label: label.to_string(), pipeline })
    }

    /// Runs `kernel` for at least `count` invocations with buffers bound at
    /// group 0, bindings 0, 1, 2, ... Kernels must ignore indices `>= count`.
    pub async fn run(&self, kernel: &Kernel, buffers: &[&wgpu::Buffer], count: u64) -> anyhow::Result<()> {
        if count == 0 {
            return Ok(());
        }
        #[cfg(not(target_arch = "wasm32"))]
        let started = web_time::Instant::now();
        #[cfg(not(target_arch = "wasm32"))]
        if self.profile.is_some() {
            self.flush();
            let _ = self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        }
        #[cfg(not(target_arch = "wasm32"))]
        let started = if self.profile.is_some() { web_time::Instant::now() } else { started };
        for buffer in buffers {
            self.peak.fetch_max(buffer.size(), std::sync::atomic::Ordering::Relaxed);
        }
        let entries: Vec<wgpu::BindGroupEntry> = buffers
            .iter()
            .enumerate()
            .map(|(n, buffer)| wgpu::BindGroupEntry { binding: n as u32, resource: buffer.as_entire_binding() })
            .collect();
        // A browser answers an error scope through its event loop; awaiting one per dispatch
        // would dominate. There the device's error callback is checked at the next readback.
        #[cfg(not(target_arch = "wasm32"))]
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&kernel.label),
            layout: &kernel.pipeline.get_bind_group_layout(0),
            entries: &entries,
        });
        let groups = count.div_ceil(WORKGROUP as u64);
        let (x, y) = if groups <= ROW_GROUPS as u64 { (groups as u32, 1) } else { (ROW_GROUPS, groups.div_ceil(ROW_GROUPS as u64) as u32) };
        if y > self.limits.max_compute_workgroups_per_dimension {
            return Err(anyhow!("kernel {}: {count} invocations exceed one dispatch", kernel.label));
        }
        self.record(|encoder| {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&kernel.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(x, y, 1);
        });
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(error) = scope.pop().await {
            return Err(anyhow!("kernel {}: {error}", kernel.label));
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(profile) = &self.profile {
            self.flush();
            let _ = self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
            let elapsed = started.elapsed().as_secs_f64();
            let mut table = profile.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let entry = table.entry(kernel.label.clone()).or_default();
            entry.0 += elapsed;
            entry.1 += 1;
        }
        Ok(())
    }

    /// Seconds and dispatches per kernel since the last report (`CRISP3DS_GPU_PROFILE=1`, native), slowest first.
    pub fn profile_report(&self) -> Option<String> {
        let profile = self.profile.as_ref()?;
        let mut table = profile.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut rows: Vec<_> = std::mem::take(&mut *table).into_iter().collect();
        rows.sort_by(|a, b| b.1 .0.total_cmp(&a.1 .0));
        Some(rows.iter().map(|(name, (seconds, n))| format!("{name} {seconds:.3}s/{n}")).collect::<Vec<_>>().join(", "))
    }
}

/// A readback in flight ([`Gpu::begin_read`]).
pub struct PendingRead<T> {
    staging: wgpu::Buffer,
    mapped: Mapped,
    count: usize,
    bytes: u64,
    #[cfg(not(target_arch = "wasm32"))]
    index: wgpu::SubmissionIndex,
    #[cfg(target_arch = "wasm32")]
    index: (),
    element: std::marker::PhantomData<T>,
}

/// Completion of a buffer mapping, as a future.
#[derive(Clone, Default)]
struct Mapped(std::sync::Arc<std::sync::Mutex<MappedState>>);

#[derive(Default)]
struct MappedState {
    result: Option<Result<(), wgpu::BufferAsyncError>>,
    waker: Option<std::task::Waker>,
}

impl Mapped {
    fn complete(&self, result: Result<(), wgpu::BufferAsyncError>) {
        let mut state = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        state.result = Some(result);
        if let Some(waker) = state.waker.take() {
            waker.wake();
        }
    }
}

impl std::future::Future for Mapped {
    type Output = Result<(), wgpu::BufferAsyncError>;

    fn poll(self: std::pin::Pin<&mut Self>, context: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
        let mut state = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        match state.result.take() {
            Some(result) => std::task::Poll::Ready(result),
            None => {
                state.waker = Some(context.waker().clone());
                std::task::Poll::Pending
            }
        }
    }
}

/// Runs a future of this crate to completion on the calling thread. Native only: a browser awaits instead.
#[cfg(not(target_arch = "wasm32"))]
pub fn block_on<F: std::future::Future>(future: F) -> F::Output {
    pollster::block_on(future)
}

pub struct Kernel {
    label: String,
    pipeline: wgpu::ComputePipeline,
}

/// Packs booleans into 32-bit words, least significant bit first.
pub fn pack_bits(values: impl ExactSizeIterator<Item = bool>) -> Vec<u32> {
    let mut words = vec![0u32; values.len().div_ceil(32).max(1)];
    for (n, value) in values.enumerate() {
        if value {
            words[n / 32] |= 1 << (n % 32);
        }
    }
    words
}

/// True when GPU tests should run (`CRISP3DS_GPU_TESTS=1`); they need an adapter.
pub fn tests_enabled() -> bool {
    std::env::var("CRISP3DS_GPU_TESTS").map(|v| v == "1").unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use wgpu::naga;

    const SHADERS: [(&str, &str); 38] = [
        ("carve", include_str!("../shaders/carve.wgsl")),
        ("cover", include_str!("../shaders/cover.wgsl")),
        ("fuse", include_str!("../shaders/fuse.wgsl")),
        ("warp", include_str!("../shaders/warp.wgsl")),
        ("ncc_rows", include_str!("../shaders/ncc_rows.wgsl")),
        ("ncc_columns", include_str!("../shaders/ncc_columns.wgsl")),
        ("aggregate", include_str!("../shaders/aggregate.wgsl")),
        ("peak", include_str!("../shaders/peak.wgsl")),
        ("patchmatch", include_str!("../shaders/patchmatch.wgsl")),
        ("slanted", include_str!("../shaders/slanted.wgsl")),
        ("surface_matmul", include_str!("../shaders/surface_matmul.wgsl")),
        ("surface_encode", include_str!("../shaders/surface_encode.wgsl")),
        ("surface_encode_backward", include_str!("../shaders/surface_encode_backward.wgsl")),
        ("surface_activate", include_str!("../shaders/surface_activate.wgsl")),
        ("surface_activate_backward", include_str!("../shaders/surface_activate_backward.wgsl")),
        ("surface_colour_input", include_str!("../shaders/surface_colour_input.wgsl")),
        ("surface_combine", include_str!("../shaders/surface_combine.wgsl")),
        ("surface_render", include_str!("../shaders/surface_render.wgsl")),
        ("surface_reduce", include_str!("../shaders/surface_reduce.wgsl")),
        ("surface_bias_grad", include_str!("../shaders/surface_bias_grad.wgsl")),
        ("surface_variance_grad", include_str!("../shaders/surface_variance_grad.wgsl")),
        ("surface_normalise", include_str!("../shaders/surface_normalise.wgsl")),
        ("surface_normalise_backward", include_str!("../shaders/surface_normalise_backward.wgsl")),
        ("surface_adam", include_str!("../shaders/surface_adam.wgsl")),
        ("surface_band", include_str!("../shaders/surface_band.wgsl")),
        ("surface_coarse", include_str!("../shaders/surface_coarse.wgsl")),
        ("surface_round", include_str!("../shaders/surface_round.wgsl")),
        ("surface_fit", include_str!("../shaders/surface_fit.wgsl")),
        ("splat_project", include_str!("../shaders/splat_project.wgsl")),
        ("splat_scan", include_str!("../shaders/splat_scan.wgsl")),
        ("splat_radix_count", include_str!("../shaders/splat_radix_count.wgsl")),
        ("splat_radix_scatter", include_str!("../shaders/splat_radix_scatter.wgsl")),
        ("splat_iota", include_str!("../shaders/splat_iota.wgsl")),
        ("splat_gather", include_str!("../shaders/splat_gather.wgsl")),
        ("splat_emit", include_str!("../shaders/splat_emit.wgsl")),
        ("splat_ranges", include_str!("../shaders/splat_ranges.wgsl")),
        ("splat_raster", include_str!("../shaders/splat_raster.wgsl")),
        ("splat_raster_backward", include_str!("../shaders/splat_raster_backward.wgsl")),
    ];

    /// Every kernel parses and validates without a GPU, uses plain WebGPU
    /// features only and stays within the default limit of 8 storage buffers.
    #[test]
    fn kernels_are_valid_webgpu() {
        for (name, source) in SHADERS {
            // The surface's matrix product is generated per mode from its template.
            let source = if name == "surface_matmul" { crate::surface::matmul_source(0, 64) } else { source.to_string() };
            let source = format!("const ROW: u32 = {}u;\n{source}", super::ROW);
            let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(&source)));
            let mut validator = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty());
            validator.validate(&module).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            let storage = module.global_variables.iter().filter(|(_, v)| matches!(v.space, naga::AddressSpace::Storage { .. })).count();
            assert!(storage <= 8, "{name} binds {storage} storage buffers");
            assert!(!module.entry_points.is_empty(), "{name} has no entry point");
            for entry in &module.entry_points {
                assert_eq!(entry.stage, naga::ShaderStage::Compute, "{name}");
                assert_eq!(entry.workgroup_size, [super::WORKGROUP, 1, 1], "{name}");
            }
        }
    }

    /// Names the adapter the GPU tests run on (CRISP3DS_GPU_TESTS=1), for CI logs.
    #[test]
    fn adapter() {
        if !super::tests_enabled() {
            return;
        }
        let gpu = super::Gpu::new().expect("a GPU adapter");
        println!("adapter: {} ({:?}, {:?})", gpu.info.name, gpu.info.backend, gpu.info.device_type);
        assert!(gpu.limits.max_storage_buffers_per_shader_stage >= 8);
    }

    #[test]
    fn bits_pack_least_significant_first() {
        let mut flags = vec![false; 34];
        (flags[0], flags[2], flags[33]) = (true, true, true);
        assert_eq!(super::pack_bits(flags.into_iter()), vec![0b101, 0b10]);
        assert_eq!(super::pack_bits(Vec::<bool>::new().into_iter()), vec![0]);
    }
}
