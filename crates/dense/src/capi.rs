//! C ABI of the run driver, for Swift, Kotlin (JNI/NDK) and C++ callers.
//! Built only with the `capi` feature; see `include/crispmesh_dense.h`.
//!
//! A run is started from a JSON options string (the fields of
//! [`crate::run::RunOptions`]) and executes on its own thread. The caller polls
//! for new events and the final report, may cancel, and frees the handle.
//! Strings returned by this library are freed with `crispmesh_string_free`.
//! This is the only module of the crate with `unsafe` code beyond byte casts:
//! it dereferences the pointers the caller passes in.

use std::ffi::{c_char, CStr, CString};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use serde_json::{json, Value};

use crate::run::{run, RunOptions};

/// Opaque handle of one run.
pub struct CrispMeshRun {
    cancel: Arc<AtomicBool>,
    events: Arc<Mutex<Vec<Value>>>,
    outcome: Arc<Mutex<Option<Result<Value, String>>>>,
    thread: Option<JoinHandle<()>>,
}

fn into_c(text: String) -> *mut c_char {
    CString::new(text.replace('\0', " ")).map(CString::into_raw).unwrap_or(std::ptr::null_mut())
}

fn start(options: &str) -> Result<CrispMeshRun, String> {
    let options: RunOptions = serde_json::from_str(options).map_err(|e| format!("options: {e}"))?;
    options.configuration().map_err(|e| format!("{e:#}"))?;
    let cancel = Arc::new(AtomicBool::new(false));
    let events = Arc::new(Mutex::new(Vec::new()));
    let outcome = Arc::new(Mutex::new(None));
    let (flag, sink, result) = (cancel.clone(), events.clone(), outcome.clone());
    let thread = std::thread::Builder::new()
        .name("crispmesh-run".into())
        .spawn(move || {
            let observer: crate::events::Observer =
                Arc::new(move |event: &Value| sink.lock().unwrap_or_else(|p| p.into_inner()).push(event.clone()));
            let finished = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&options, Some(observer), Some(flag))));
            let value = match finished {
                Ok(Ok(report)) => Ok(report),
                Ok(Err(error)) => Err(format!("{error:#}")),
                Err(_) => Err("the run panicked".to_string()),
            };
            *result.lock().unwrap_or_else(|p| p.into_inner()) = Some(value);
        })
        .map_err(|e| format!("cannot start a thread: {e}"))?;
    Ok(CrispMeshRun { cancel, events, outcome, thread: Some(thread) })
}

fn poll(run: &CrispMeshRun) -> Value {
    let events: Vec<Value> = std::mem::take(&mut *run.events.lock().unwrap_or_else(|p| p.into_inner()));
    let outcome = run.outcome.lock().unwrap_or_else(|p| p.into_inner());
    let cancelled = run.cancel.load(Ordering::Relaxed);
    match &*outcome {
        None => json!({"finished": false, "status": "running", "events": events, "report": null, "error": null}),
        Some(Ok(report)) => json!({"finished": true, "status": "complete", "events": events, "report": report, "error": null}),
        Some(Err(error)) => {
            json!({"finished": true, "status": if cancelled { "cancelled" } else { "failed" }, "events": events, "report": null, "error": error})
        }
    }
}

/// Version of the library, a static string (do not free).
#[no_mangle]
pub extern "C" fn crispmesh_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr().cast()
}

/// Starts a run. Returns null when the options are invalid; `*error`, if `error`
/// is not null, then holds a message to free with `crispmesh_string_free`.
///
/// # Safety
/// `options_json` must be a valid NUL-terminated UTF-8 string; `error` must be null or point to writable storage for one pointer.
#[no_mangle]
pub unsafe extern "C" fn crispmesh_run_start(options_json: *const c_char, error: *mut *mut c_char) -> *mut CrispMeshRun {
    let fail = |message: String| {
        if !error.is_null() {
            // SAFETY: the caller guarantees `error` points to storage for one pointer.
            unsafe { *error = into_c(message) };
        }
        std::ptr::null_mut()
    };
    if options_json.is_null() {
        return fail("options are null".to_string());
    }
    // SAFETY: the caller guarantees a valid NUL-terminated string.
    let text = match unsafe { CStr::from_ptr(options_json) }.to_str() {
        Ok(text) => text,
        Err(_) => return fail("options are not UTF-8".to_string()),
    };
    match start(text) {
        Ok(run) => Box::into_raw(Box::new(run)),
        Err(message) => fail(message),
    }
}

/// New events since the last call and the state of the run, as a JSON object
/// `{"finished", "status", "events", "report", "error"}`. Free the string with `crispmesh_string_free`.
///
/// # Safety
/// `run` must be a handle from `crispmesh_run_start` that was not freed.
#[no_mangle]
pub unsafe extern "C" fn crispmesh_run_poll(run: *mut CrispMeshRun) -> *mut c_char {
    // SAFETY: the caller guarantees a live handle.
    match unsafe { run.as_ref() } {
        Some(run) => into_c(poll(run).to_string()),
        None => std::ptr::null_mut(),
    }
}

/// Asks the run to stop; it ends with status `cancelled` at the next view or stage boundary.
///
/// # Safety
/// `run` must be a handle from `crispmesh_run_start` that was not freed.
#[no_mangle]
pub unsafe extern "C" fn crispmesh_run_cancel(run: *mut CrispMeshRun) {
    // SAFETY: the caller guarantees a live handle.
    if let Some(run) = unsafe { run.as_ref() } {
        run.cancel.store(true, Ordering::Relaxed);
    }
}

/// Cancels the run if it is still going, waits for its thread and releases the handle.
///
/// # Safety
/// `run` must be null or a handle from `crispmesh_run_start`; it must not be used afterwards.
#[no_mangle]
pub unsafe extern "C" fn crispmesh_run_free(run: *mut CrispMeshRun) {
    if run.is_null() {
        return;
    }
    // SAFETY: the caller hands back the box created by `crispmesh_run_start`.
    let mut run = unsafe { Box::from_raw(run) };
    run.cancel.store(true, Ordering::Relaxed);
    if let Some(thread) = run.thread.take() {
        let _ = thread.join();
    }
}

/// Frees a string returned by this library.
///
/// # Safety
/// `text` must be null or a string returned by this library that was not freed.
#[no_mangle]
pub unsafe extern "C" fn crispmesh_string_free(text: *mut c_char) {
    if !text.is_null() {
        // SAFETY: the caller hands back a string created by `CString::into_raw` here.
        drop(unsafe { CString::from_raw(text) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_options_are_refused_with_a_message() {
        let mut error: *mut c_char = std::ptr::null_mut();
        let options = CString::new(r#"{"output": "x", "inputs": "y", "settings": {"windows": [4]}}"#).unwrap();
        // SAFETY: valid string and error slot.
        let run = unsafe { crispmesh_run_start(options.as_ptr(), &mut error) };
        assert!(run.is_null() && !error.is_null());
        // SAFETY: `error` was set by the library.
        let message = unsafe { CStr::from_ptr(error) }.to_str().unwrap().to_string();
        assert!(message.contains("windows"), "{message}");
        // SAFETY: freeing what the library returned; null handles are accepted.
        unsafe {
            crispmesh_string_free(error);
            crispmesh_run_free(std::ptr::null_mut());
            assert!(crispmesh_run_poll(std::ptr::null_mut()).is_null());
        }
        // SAFETY: a static NUL-terminated string.
        assert_eq!(unsafe { CStr::from_ptr(crispmesh_version()) }.to_str().unwrap(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn a_run_that_cannot_start_reports_failure_through_poll() {
        let root = std::env::temp_dir().join(format!("crispmesh-capi-{}", std::process::id()));
        let _ = crate::storage::remove_dir_all(&root);
        let options = CString::new(json!({"output": root.join("run"), "inputs": root.join("missing")}).to_string()).unwrap();
        // SAFETY: valid string; null error slot is allowed.
        let run = unsafe { crispmesh_run_start(options.as_ptr(), std::ptr::null_mut()) };
        assert!(!run.is_null());
        let mut state = Value::Null;
        for _ in 0..600 {
            // SAFETY: live handle; the returned string is freed below.
            let text = unsafe { crispmesh_run_poll(run) };
            state = serde_json::from_str(unsafe { CStr::from_ptr(text) }.to_str().unwrap()).unwrap();
            unsafe { crispmesh_string_free(text) };
            if state["finished"] == true {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(state["status"], "failed", "{state}");
        assert!(state["error"].as_str().unwrap().contains("cameras.json"), "{state}");
        // SAFETY: live handle, freed once.
        unsafe { crispmesh_run_free(run) };
        let _ = crate::storage::remove_dir_all(&root);
    }
}
