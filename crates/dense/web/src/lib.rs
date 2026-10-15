//! Browser bindings of the dense pipeline: WebAssembly for the CPU passes,
//! WebGPU for the kernels, an in-memory file tree instead of a disk.
//!
//! The host (a page or, better, a worker) puts the input files into the tree,
//! starts a run with a JSON options string whose paths point into the tree,
//! receives every event as an object through a callback, and reads output files
//! out of the tree, for example when an `artifact` event names one. See
//! `crispmesh-dense.js` for a wrapper that does this from a map of files.
//!
//! Everything runs on the calling JavaScript thread. GPU work is awaited, so the
//! thread returns to its event loop at every readback (once per view while
//! matching); the CPU passes between them do not yield.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crispmesh_dense::events::Observer;
use crispmesh_dense::run::{run_async, RunOptions};
use crispmesh_dense::storage;
use wasm_bindgen::prelude::*;

thread_local! {
    /// Event callbacks of the runs in progress, by run number.
    static CALLBACKS: RefCell<BTreeMap<u32, js_sys::Function>> = const { RefCell::new(BTreeMap::new()) };
    static NEXT: RefCell<u32> = const { RefCell::new(1) };
    /// Supplies files the host keeps in JavaScript memory.
    static SOURCE: RefCell<Option<js_sys::Function>> = const { RefCell::new(None) };
    /// Lists folders the host keeps in JavaScript memory.
    static LISTER: RefCell<Option<js_sys::Function>> = const { RefCell::new(None) };
}

#[wasm_bindgen(start)]
fn start() {
    console_error_panic_hook::set_once();
}

/// `initThreadPool(n)` (threaded package only): starts rayon's pool of `n` Web Workers; await it
/// once before the first run. Needs cross-origin isolation (`self.crossOriginIsolated`).
#[cfg(feature = "threads")]
pub use wasm_bindgen_rayon::init_thread_pool;

/// Whether this package was built with threads (shared memory and a worker pool).
#[wasm_bindgen]
pub fn threaded() -> bool {
    cfg!(feature = "threads")
}

/// Version of the engine.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Name, group, meaning, kind and default of every setting: `{"settings": [...]}` as JSON text.
#[wasm_bindgen(js_name = settingsSchema)]
pub fn settings_schema() -> String {
    crispmesh_dense::config::settings_schema().to_string()
}

/// How a run may start in a browser, as JSON text: the start points (photos, an
/// inputs directory), their fields, stages and the providers this build can run
/// (no external programs), with their options. See `crispmesh_dense::run::describe`.
#[wasm_bindgen]
pub fn describe() -> String {
    crispmesh_dense::run::describe().to_string()
}

/// The default settings as JSON text.
#[wasm_bindgen(js_name = defaultSettings)]
pub fn default_settings() -> String {
    serde_json::to_string(&crispmesh_dense::config::DenseConfig::default()).unwrap_or_default()
}

/// Stores a file in the in-memory tree.
#[wasm_bindgen(js_name = putFile)]
pub fn put_file(path: &str, bytes: Vec<u8>) -> Result<(), JsError> {
    storage::write_owned(path, bytes).map_err(|e| JsError::new(&e.to_string()))
}

/// A file of the in-memory tree, or undefined.
#[wasm_bindgen(js_name = getFile)]
pub fn get_file(path: &str) -> Option<Vec<u8>> {
    storage::read(path).ok()
}

/// Every file under a directory of the tree: `[[path, bytes], ...]` as JSON text.
#[wasm_bindgen(js_name = listFiles)]
pub fn list_files(directory: &str) -> String {
    serde_json::to_string(&storage::memory_files(directory)).unwrap_or_default()
}

/// Registers `source(path, probe)`: called for files that are not in the tree.
/// It returns a `Uint8Array` with the file (or, when `probe` is true, any
/// truthy value if the file exists) and `undefined` otherwise. Large inputs can
/// then stay in JavaScript memory instead of WebAssembly memory. `undefined`
/// or `null` removes the source.
#[wasm_bindgen(js_name = setFileSource)]
pub fn set_file_source(source: Option<js_sys::Function>) {
    let present = source.is_some();
    SOURCE.with(|slot| *slot.borrow_mut() = source);
    storage::set_memory_source(present.then(|| {
        Box::new(|path: &str, probe: bool| {
            let function = SOURCE.with(|slot| slot.borrow().clone())?;
            let value = function.call2(&JsValue::NULL, &JsValue::from_str(path), &JsValue::from_bool(probe)).ok()?;
            if probe {
                return value.is_truthy().then(Vec::new);
            }
            value.is_instance_of::<js_sys::Uint8Array>().then(|| js_sys::Uint8Array::from(value).to_vec())
        }) as storage::Source
    }));
}

/// Registers `lister(directory)`: called when a directory of the tree is listed.
/// It returns an array with the names of the files the host supplies directly
/// inside that directory (through the file source), or `undefined` when it
/// supplies none there. A folder of photos kept in JavaScript memory is then
/// listed like a stored one. `undefined` or `null` removes the lister.
#[wasm_bindgen(js_name = setFileLister)]
pub fn set_file_lister(lister: Option<js_sys::Function>) {
    let present = lister.is_some();
    LISTER.with(|slot| *slot.borrow_mut() = lister);
    storage::set_memory_lister(present.then(|| {
        Box::new(|directory: &str| {
            let function = LISTER.with(|slot| slot.borrow().clone())?;
            let value = function.call1(&JsValue::NULL, &JsValue::from_str(directory)).ok()?;
            if !js_sys::Array::is_array(&value) {
                return None;
            }
            Some(js_sys::Array::from(&value).iter().filter_map(|name| name.as_string()).collect())
        }) as storage::Lister
    }));
}

/// Removes one file of the tree (a file the host has taken over and now supplies through the file source).
#[wasm_bindgen(js_name = removeFile)]
pub fn remove_file(path: &str) {
    let _ = storage::remove_file(path);
}

/// Removes a directory of the tree with everything in it.
#[wasm_bindgen(js_name = removeTree)]
pub fn remove_tree(directory: &str) {
    let _ = storage::remove_dir_all(directory);
}

/// Bytes held by the files of the tree.
#[wasm_bindgen(js_name = storedBytes)]
pub fn stored_bytes() -> f64 {
    storage::memory_bytes() as f64
}

/// One run. Create it, keep it to cancel, call `start` once.
#[wasm_bindgen]
pub struct Run {
    cancel: Arc<AtomicBool>,
}

impl Default for Run {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl Run {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Run {
        Run { cancel: Arc::new(AtomicBool::new(false)) }
    }

    /// Asks the run to stop; it ends with status `cancelled` at the next view or stage boundary.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Runs the pipeline. `options` is JSON text with the fields of `RunOptions`
    /// (`output` and `inputs` are paths in the tree). `on_event` receives every
    /// event as an object, the same objects as the lines of `events.jsonl`.
    /// Resolves with the content of `pipeline.json` as JSON text; rejects with
    /// the error message when the run fails or is cancelled.
    pub async fn start(&self, options: String, on_event: Option<js_sys::Function>) -> Result<String, JsError> {
        let options: RunOptions = serde_json::from_str(&options).map_err(|e| JsError::new(&format!("options: {e}")))?;
        let id = NEXT.with(|next| {
            let mut next = next.borrow_mut();
            *next += 1;
            *next
        });
        if let Some(callback) = on_event {
            CALLBACKS.with(|callbacks| callbacks.borrow_mut().insert(id, callback));
        }
        // The callback lives in thread-local storage: the observer itself must be shareable between threads.
        let observer: Observer = Arc::new(move |event: &serde_json::Value| {
            let callback = CALLBACKS.with(|callbacks| callbacks.borrow().get(&id).cloned());
            if let Some(callback) = callback {
                if let Ok(object) = js_sys::JSON::parse(&event.to_string()) {
                    let _ = callback.call1(&JsValue::NULL, &object);
                }
            }
        });
        let result = run_async(&options, Some(observer), Some(self.cancel.clone())).await;
        CALLBACKS.with(|callbacks| callbacks.borrow_mut().remove(&id));
        match result {
            Ok(report) => Ok(report.to_string()),
            Err(error) => Err(JsError::new(&format!("{error:#}"))),
        }
    }
}
