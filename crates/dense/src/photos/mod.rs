//! Photo front stage: from a folder of turntable photos to the dense inputs
//! directory. Port of `scripts/turntable_mesh/photos_to_inputs.py` and of the
//! modules it calls (`silhouette_cleanup.py`, `alicevision_cameras.py`,
//! `mve_full/prepare.py`'s contrast profile).
//!
//! Masks and cameras come from interchangeable providers (`docs/ARCHITECTURE.md`);
//! undistortion, gates and the scene are done here for all of them.
//!
//! | Module | Content |
//! | --- | --- |
//! | `options`, `providers` | command line, resolved configuration, the provider table |
//! | `staging`, `coarse` | capture order, `capture_NNNN.png` copies, coarse dark-object masks |
//! | `masks` | mask providers: `threshold`, `import`, `external-sam` |
//! | `sam` | mask provider `sam`: SAM 2.1 in this process (prompts, selection, backends) |
//! | `cleanup`, `sheets` | dark-hole cleanup, published masks, contact sheet, sparse overlay |
//! | `contrast` | gamma and CLAHE contrast images (OpenCV's 8-bit Lab and CLAHE) |
//! | `calibration` | lens file, scaling, the locked AliceVision intrinsic |
//! | `cameras` | camera providers: `alicevision`, `colmap`, `import` |
//! | `markers` | camera provider `markers`: the printed mat, its detection, poses in millimetres |
//! | `solution` | a camera solution in neutral form; readers for `.sfm` and COLMAP models |
//! | `audit`, `ring` | camera audit, ring statistics, gate decision |
//! | `scene_writer` | undistortion of photos and masks, the scene directory |
//! | `process`, `run` | bounded child processes; the two stages, events, `frontend.json` |
//!
//! Not ported: AliceVision and COLMAP (external programs, called as child
//! processes). SAM 2.1 runs either in this process (`--masks sam`, in builds
//! with a backend feature such as `sam-onnx`) or through the reference
//! `segment.py` in an external interpreter (`--masks external-sam`).

/// The most photos one front-end run takes, shared by all capture layouts and camera imports.
pub const MOST_PHOTOS: usize = 96 * 16;

pub mod audit;
pub mod background;
pub mod calibration;
pub mod cleanup;
pub mod coarse;
pub mod contrast;
pub mod fs;
pub mod incremental;
pub mod markers;
pub mod option_table;
pub mod options;
mod p3p;
pub mod providers;
#[cfg(not(target_arch = "wasm32"))]
pub mod rendered;
pub mod ring;
pub mod sam;
pub mod scene_writer;
pub mod sheets;
pub mod solution;
pub mod staging;
pub mod turntable;
pub mod util;

// The modules below start external programs where a provider needs one. They build for every target:
// in a browser (and with an in-memory run directory) only the providers without external programs are
// offered and run (`providers::Provider::external`), and file access goes through `fs` (crate::storage).
pub mod availability;
pub mod cameras;
pub mod masks;
pub mod process;
pub mod run;

/// Entry point of the `photos` subcommand.
#[cfg(not(target_arch = "wasm32"))]
pub fn command(arguments: &[String]) -> std::process::ExitCode {
    use std::process::ExitCode;
    if arguments.iter().any(|a| a == "--help" || a == "-h") {
        println!("{}", options::USAGE);
        return ExitCode::SUCCESS;
    }
    if arguments.iter().any(|a| a == "--list-providers") {
        println!("{}", serde_json::to_string_pretty(&providers::listing()).unwrap_or_default());
        return ExitCode::SUCCESS;
    }
    if arguments.iter().any(|a| a == "--list-options") {
        println!("{}", serde_json::to_string_pretty(&option_table::listing()).unwrap_or_default());
        return ExitCode::SUCCESS;
    }
    let usage = |error: anyhow::Error| {
        eprintln!("crispmesh-dense photos: {error:#}\n(crispmesh-dense photos --help lists the options)");
        ExitCode::from(2)
    };
    let options = match options::resolve(arguments, &|name| std::env::var(name).ok()) {
        Ok(options) => options,
        Err(error) => return usage(error),
    };
    if options.output.exists() {
        return usage(anyhow::anyhow!("output exists: {}", options.output.display()));
    }
    let checked = masks::mask_provider(&options).check(&options).and_then(|()| {
        if options.stop_after_masks {
            Ok(())
        } else {
            cameras::camera_provider(&options).check(&options)
        }
    });
    if let Err(error) = checked {
        return usage(error);
    }
    let events_path = options.events.clone().unwrap_or_else(|| options.output.join("events.jsonl"));
    let events = crate::events::EventLog::new(Some(&events_path), "cameras");
    match run::run(&options, &events, &events_path, None) {
        Ok(finished) => {
            println!("{}", serde_json::to_string_pretty(&run::summary(&finished.report)).unwrap_or_default());
            ExitCode::from(finished.code)
        }
        Err(error) => {
            eprintln!("crispmesh-dense photos: {error:#}");
            ExitCode::from(1)
        }
    }
}
