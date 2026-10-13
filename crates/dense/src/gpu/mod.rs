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
#[cfg(target_arch = "wasm32")]
const BATCH: usize = usize::MAX;

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
    #[cfg(target_arch = "wasm32")]
    pending: std::sync::Mutex<(Option<wgpu::CommandEncoder>, usize)>,
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
            #[cfg(target_arch = "wasm32")]
            pending: std::sync::Mutex::new((None, 0)),
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

    /// Records into the pending encoder (browser) or submits at once (native).
    fn record(&self, commands: impl FnOnce(&mut wgpu::CommandEncoder)) {
        #[cfg(target_arch = "wasm32")]
        {
            let mut pending = self.pending.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            commands(pending.0.get_or_insert_with(|| self.device.create_command_encoder(&Default::default())));
            pending.1 += 1;
            if pending.1 >= BATCH {
                pending.1 = 0;
                if let Some(encoder) = pending.0.take() {
                    self.queue.submit([encoder.finish()]);
                }
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut encoder = self.device.create_command_encoder(&Default::default());
            commands(&mut encoder);
            self.queue.submit([encoder.finish()]);
        }
    }

    /// Submits what was recorded (nothing to do natively).
    pub fn flush(&self) {
        #[cfg(target_arch = "wasm32")]
        {
            let mut pending = self.pending.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            pending.1 = 0;
            if let Some(encoder) = pending.0.take() {
                self.queue.submit([encoder.finish()]);
            }
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
        Ok(())
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

    const SHADERS: [(&str, &str); 10] = [
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
    ];

    /// Every kernel parses and validates without a GPU, uses plain WebGPU
    /// features only and stays within the default limit of 8 storage buffers.
    #[test]
    fn kernels_are_valid_webgpu() {
        for (name, source) in SHADERS {
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
