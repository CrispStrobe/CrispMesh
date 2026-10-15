//! Surface extraction: a closed mesh from fused evidence bounded by the silhouette hull.
//!
//! Port of `scripts/turntable_mesh/tsdf_hull_mesh.py`. Observed voxels use the
//! confidence-smoothed averaged truncated signed distance; unobserved hull voxels near
//! evidence take its extrapolation; the rest fall back to the hull's own signed distance, so
//! the surface there is the silhouette bound. Nothing outside the hull can be surface.

pub mod cubes;
pub mod edt;
pub mod field;
pub mod gaussian;
pub mod grid;
pub mod settings;
pub mod surface;
#[cfg(test)]
pub(crate) mod testdata;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use web_time::Instant;

use anyhow::{anyhow, bail, ensure, Context, Result};
use serde::Serialize;
use serde_json::json;

use crate::config::DenseConfig;
use crate::events::EventLog;
use crate::stl;
use field::{Volume, PAD};
use settings::MeshSettings;

pub const TAUBIN_LAMBDA: f64 = 0.5;
pub const TAUBIN_MU: f64 = -0.53;
pub const STL_HEADER: &str = "crispmesh tsdf hull mesh";

/// Content of `result.json`, keys in the order the reference writes them.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub flat_base_applied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hull_fraction_below_support: Option<f64>,
    pub observed_hull_fraction: f64,
    pub extrapolated_hull_fraction: f64,
    pub hull_overshoot_voxels: f64,
    pub components: usize,
    pub discarded_faces: usize,
    pub vertices: usize,
    pub triangles: usize,
    pub boundary_edges: usize,
    pub nonmanifold_edges: usize,
    pub genus: i64,
    pub signed_volume: f64,
    pub closed: bool,
    pub configuration: MeshSettings,
    pub reference_used: bool,
    pub physical_scale_established: bool,
    pub seconds: f64,
}

/// Meshes `volume_path` into the new directory `output` (`mesh.stl`, `result.json`).
///
/// `step` above 1 extracts a coarse preview: fewer triangles, no mesh smoothing.
/// `threads` bounds the worker threads of the numerical passes.
pub fn run(
    volume_path: &Path,
    output: &Path,
    config: &DenseConfig,
    step: usize,
    events: &EventLog,
    label: Option<&str>,
    threads: usize,
) -> Result<Report> {
    run_with(volume_path, output, config, step, events, label, threads, &crate::control::Control::none())
}

/// [`run`] that can be stopped: `control` is checked before every pass over the
/// volume (distance transform, each blur), before the iso-surface, before every
/// smoothing cycle and before the mesh is written. A stopped stage returns
/// `control::Stopped` and leaves no output directory.
#[allow(clippy::too_many_arguments)]
pub fn run_with(
    volume_path: &Path,
    output: &Path,
    config: &DenseConfig,
    step: usize,
    events: &EventLog,
    label: Option<&str>,
    threads: usize,
    control: &crate::control::Control,
) -> Result<Report> {
    run_source(Source::File(volume_path), output, config, step, events, label, threads, control)
}

/// [`run_with`] on a volume held in memory (handed over by the stereo stage of the same run):
/// the same mesh as from its `volume.npz`, without encoding and decoding the file.
#[allow(clippy::too_many_arguments)]
pub fn run_volume(
    volume: Volume,
    output: &Path,
    config: &DenseConfig,
    step: usize,
    events: &EventLog,
    label: Option<&str>,
    threads: usize,
    control: &crate::control::Control,
) -> Result<Report> {
    run_source(Source::Memory(Box::new(volume)), output, config, step, events, label, threads, control)
}

/// A coarse preview from a volume in memory: the volume is coarsened by `step`
/// (`Volume::coarsened`) and meshed cell by cell, instead of meshing every
/// `step`-th cell of the full grid: the same coarseness for about `step`^3
/// times less memory. Reported as a preview surface like [`run_with`] with that step.
#[allow(clippy::too_many_arguments)]
pub fn run_preview(
    volume: Volume,
    output: &Path,
    config: &DenseConfig,
    step: usize,
    events: &EventLog,
    label: Option<&str>,
    threads: usize,
    control: &crate::control::Control,
) -> Result<Report> {
    run_source(Source::Coarse(Box::new(volume)), output, config, step, events, label, threads, control)
}

enum Source<'a> {
    File(&'a Path),
    Memory(Box<Volume>),
    Coarse(Box<Volume>),
}

#[allow(clippy::too_many_arguments)]
fn run_source(
    volume_path: Source,
    output: &Path,
    config: &DenseConfig,
    step: usize,
    events: &EventLog,
    label: Option<&str>,
    threads: usize,
    control: &crate::control::Control,
) -> Result<Report> {
    settings::validate(config)?;
    ensure!(step >= 1, "step must be at least 1");
    ensure!(!crate::storage::exists(output), "output directory already exists: {}", output.display());
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        crate::storage::create_dir_all(parent).with_context(|| format!("cannot create {}", parent.display()))?;
    }
    crate::storage::create_dir(output).with_context(|| format!("cannot create output directory {}", output.display()))?;
    // Without threads (wasm32) the passes run on the calling thread.
    #[cfg(target_arch = "wasm32")]
    {
        let _ = threads;
        finish(output, extract(volume_path, output, config, step, events, label, control))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(threads.max(1)).build()?;
        finish(output, pool.install(|| extract(volume_path, output, config, step, events, label, control)))
    }
}

/// A stopped stage leaves nothing behind.
fn finish(output: &Path, result: Result<Report>) -> Result<Report> {
    if result.as_ref().is_err_and(|error| error.downcast_ref::<crate::control::Stopped>().is_some()) {
        let _ = crate::storage::remove_dir_all(output);
    }
    result
}

fn extract(
    volume_path: Source,
    output: &Path,
    config: &DenseConfig,
    step: usize,
    events: &EventLog,
    label: Option<&str>,
    control: &crate::control::Control,
) -> Result<Report> {
    let started = Instant::now();
    let (volume, grid_step) = match volume_path {
        Source::File(path) => (Volume::read(path)?, step),
        Source::Memory(volume) => (*volume, step),
        Source::Coarse(volume) => (volume.coarsened(step), 1),
    };
    control.check()?;
    let field = field::field(&volume, config, &|| control.check())?;
    control.check()?;
    let grid_mesh = cubes::extract(&field.value, field.dims, grid_step);
    control.check()?;
    let field_report = field.report;
    drop(field.value);
    // Views that do not go around the object: which grid vertices lie at voxels with measured or
    // extrapolated evidence. Surface elsewhere (the hull's unseen back) is left out below.
    let seen: Option<Vec<bool>> = field.known.as_ref().map(|known| {
        let dims = field.dims;
        grid_mesh
            .vertices
            .iter()
            .map(|v| {
                let base = v.map(|c| c.floor().max(0.0) as usize);
                (0..8).any(|corner| {
                    let p = [0, 1, 2].map(|a| (base[a] + (corner >> a & 1)).min(dims[a] - 1));
                    known[(p[0] * dims[1] + p[1]) * dims[2] + p[2]]
                })
            })
            .collect()
    });
    drop(field.known);

    // Grid coordinates to scene coordinates, in float32 like the reference (voxel centres
    // sit at half-integers of the unpadded grid).
    let voxel = volume.voxel as f32;
    let origin_f32 = volume.origin.to_f32();
    let origin_f64 = volume.origin.to_f64();
    let single = volume.origin.is_f32();
    let vertices: Vec<surface::Vertex> = grid_mesh
        .vertices
        .iter()
        .map(|v| {
            std::array::from_fn(|axis| {
                let scaled = (v[axis] - PAD as f32 + 0.5) * voxel;
                if single {
                    f64::from(scaled + origin_f32[axis])
                } else {
                    f64::from(scaled) + origin_f64[axis]
                }
            })
        })
        .collect();
    let (part, origin) = surface::largest_component_with(&vertices, &grid_mesh.faces)?;
    drop((vertices, grid_mesh));
    let (mut vertices, mut faces) = (part.vertices, part.faces);
    // An open mesh has no meaningful signed volume: orient the closed component first, then cut.
    let mut oriented = false;
    if let Some(seen) = seen {
        if surface::topology(&vertices, &faces).signed_volume < 0.0 {
            faces.iter_mut().for_each(|f| f.swap(0, 2));
        }
        oriented = true;
        faces.retain(|f| f.iter().any(|&v| seen[origin[v as usize] as usize]));
        let open = surface::largest_component(&vertices, &faces)?;
        (vertices, faces) = (open.vertices, open.faces);
    }
    drop(origin);
    if config.mesh_taubin_cycles > 0 && step == 1 {
        // One cycle at a time (a cycle depends only on the vertices before it), so that a stop takes effect between them.
        for _ in 0..config.mesh_taubin_cycles {
            control.check()?;
            surface::taubin(&mut vertices, &faces, 1, TAUBIN_LAMBDA, TAUBIN_MU);
        }
    }
    let mut info = surface::topology(&vertices, &faces);
    if !oriented && info.signed_volume < 0.0 {
        faces.iter_mut().for_each(|f| f.swap(0, 2));
        info = surface::topology(&vertices, &faces);
    }
    control.check()?;
    let mesh_path = output.join("mesh.stl");
    stl::write_binary(&mesh_path, STL_HEADER, &vertices, &faces)?;
    let report = Report {
        flat_base_applied: field_report.flat_base_applied,
        hull_fraction_below_support: field_report.hull_fraction_below_support,
        observed_hull_fraction: field_report.observed_hull_fraction,
        extrapolated_hull_fraction: field_report.extrapolated_hull_fraction,
        hull_overshoot_voxels: field_report.hull_overshoot_voxels,
        components: part.components,
        discarded_faces: part.discarded_faces,
        vertices: info.vertices,
        triangles: info.triangles,
        boundary_edges: info.boundary_edges,
        nonmanifold_edges: info.nonmanifold_edges,
        genus: info.genus,
        signed_volume: info.signed_volume,
        closed: info.boundary_edges == 0 && info.nonmanifold_edges == 0,
        configuration: MeshSettings::from(config),
        reference_used: false,
        physical_scale_established: false,
        seconds: started.elapsed().as_secs_f64(),
    };
    crate::storage::write(output.join("result.json"), serde_json::to_string_pretty(&report)? + "\n")?;
    let (kind, default_label) = if step == 1 { ("final_mesh", "Final surface") } else { ("preview_mesh", "Preview surface") };
    // Resolved, so that the event carries the path relative to the run directory.
    let resolved = crate::storage::canonicalize(&mesh_path).unwrap_or(mesh_path);
    events.artifact(kind, &resolved, label.unwrap_or(default_label), json!({"triangles": info.triangles}))?;
    Ok(report)
}

pub const USAGE: &str = "usage: crispmesh-dense mesh --volume volume.npz --output DIR [--config config.json] [--set KEY=VALUE ...]
                           [--step N] [--events events.jsonl] [--label TEXT] [--threads N]

Extracts a closed surface from fused TSDF evidence bounded by the silhouette hull
(the native counterpart of `python -m scripts.turntable_mesh.tsdf_hull_mesh`).

  --volume FILE    volume.npz written by the stereo stage
  --output DIR     fresh directory for mesh.stl and result.json
  --config FILE    JSON with settings (a previous result.json works)
  --set KEY=VALUE  override one setting, e.g. --set mesh_taubin_cycles=0
  --step N         marching-cubes step; above 1 gives a coarse preview (default 1)
  --events FILE    append the artifact event to this run event log
  --label TEXT     label of the artifact event
  --threads N      worker threads (default 2)";

struct Arguments {
    volume: PathBuf,
    output: PathBuf,
    config: Option<PathBuf>,
    set: Vec<String>,
    step: usize,
    events: Option<PathBuf>,
    label: Option<String>,
    threads: usize,
}

fn parse(arguments: &[String]) -> Result<Arguments> {
    let (mut volume, mut output, mut config, mut events, mut label) = (None, None, None, None, None);
    let (mut set, mut step, mut threads) = (Vec::new(), 1usize, 2usize);
    let mut rest = arguments.iter();
    while let Some(argument) = rest.next() {
        let (name, inline) = match argument.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value.to_string())),
            _ => (argument.as_str(), None),
        };
        let mut value = || inline.clone().or_else(|| rest.next().cloned()).ok_or_else(|| anyhow!("{name} needs a value"));
        let count = |text: String| {
            text.parse::<usize>().ok().filter(|&n| n >= 1).ok_or_else(|| anyhow!("{name} needs a positive integer, found {text:?}"))
        };
        match name {
            "--volume" => volume = Some(PathBuf::from(value()?)),
            "--output" => output = Some(PathBuf::from(value()?)),
            "--config" => config = Some(PathBuf::from(value()?)),
            "--events" => events = Some(PathBuf::from(value()?)),
            "--label" => label = Some(value()?),
            "--set" => set.push(value()?),
            "--step" => step = count(value()?)?,
            "--threads" => threads = count(value()?)?,
            _ => bail!("unknown argument {argument:?}"),
        }
    }
    Ok(Arguments {
        volume: volume.ok_or_else(|| anyhow!("--volume is required"))?,
        output: output.ok_or_else(|| anyhow!("--output is required"))?,
        config,
        set,
        step,
        events,
        label,
        threads,
    })
}

/// `crispmesh-dense mesh ...`: prints the report as JSON, like the Python module.
pub fn command(arguments: &[String]) -> ExitCode {
    if arguments.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let parsed = match parse(arguments) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("crispmesh-dense mesh: {error}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let outcome = (|| -> Result<Report> {
        let config = settings::build(parsed.config.as_deref(), &parsed.set)?;
        // The stage name follows the reference: previews are made while matching runs.
        let stage = if parsed.step == 1 { "mesh" } else { "stereo" };
        let log = match &parsed.events {
            Some(path) => {
                let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
                let parent =
                    crate::storage::canonicalize(parent).with_context(|| format!("no directory for the event log {}", path.display()))?;
                let name = path.file_name().ok_or_else(|| anyhow!("--events needs a file name"))?;
                EventLog::new(Some(&parent.join(name)), stage)
            }
            None => EventLog::none(),
        };
        run(&parsed.volume, &parsed.output, &config, parsed.step, &log, parsed.label.as_deref(), parsed.threads)
    })();
    match outcome {
        Ok(report) => {
            println!("{}", serde_json::to_string_pretty(&report).expect("a report serialises"));
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("crispmesh-dense mesh: {error:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::npz::{self, Array, Data};

    /// A fresh scratch directory, removed when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("crispmesh-mesh-{}-{name}", std::process::id()));
            let _ = crate::storage::remove_dir_all(&path);
            crate::storage::create_dir_all(&path).unwrap();
            Scratch(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = crate::storage::remove_dir_all(&self.0);
        }
    }

    /// `sphere_volume` of `test_tsdf_hull_mesh.py`: the hull is a ball of radius 1.15, the
    /// evidence the exact truncated signed distance of a unit ball. `support` adds a support
    /// plane with normal +z ("down") at that height.
    fn sphere_volume(path: &Path, observed: bool, support: Option<f32>) {
        let (n, voxel, truncation) = (64usize, 0.05f64, 3.0f64);
        let (mut index, mut total, mut weight) = (Vec::new(), Vec::new(), Vec::new());
        for x in 0..n {
            for y in 0..n {
                for z in 0..n {
                    let centre = [x, y, z].map(|i| (i as f64 + 0.5) * voxel - n as f64 * voxel / 2.0);
                    let radius = centre.iter().map(|c| c * c).sum::<f64>().sqrt();
                    if radius < 1.15 {
                        let sdf = (radius - 1.0) / (truncation * voxel);
                        let w = if sdf > -1.0 && observed { 6.0 } else { 0.0 };
                        index.extend([x as i32, y as i32, z as i32]);
                        weight.push(w as f32);
                        total.push((sdf.clamp(-1.0, 1.0) * w) as f32);
                    }
                }
            }
        }
        let count = weight.len();
        let mut arrays = vec![
            ("index", Array::new(&[count, 3], Data::I32(index))),
            ("total", Array::new(&[count], Data::F32(total))),
            ("weight", Array::new(&[count], Data::F32(weight))),
            ("shape", Array::new(&[3], Data::I64(vec![n as i64; 3]))),
            ("origin", Array::new(&[3], Data::F32(vec![(-(n as f64) * voxel / 2.0) as f32; 3]))),
            ("voxel", Array::scalar_f32(voxel as f32)),
            ("truncation", Array::scalar_f32((truncation * voxel) as f32)),
        ];
        if let Some(height) = support {
            arrays.push(("support_point", Array::new(&[3], Data::F32(vec![0.0; 3]))));
            arrays.push(("support_down", Array::new(&[3], Data::F32(vec![0.0, 0.0, 1.0]))));
            arrays.push(("support_height", Array::scalar_f32(height)));
        }
        let entries: Vec<(&str, &Array)> = arrays.iter().map(|(name, array)| (*name, array)).collect();
        npz::write(path, &entries, true).unwrap();
    }

    fn no_taubin() -> DenseConfig {
        settings::build(None, &["mesh_taubin_cycles=0".to_string()]).unwrap()
    }

    fn corners(path: &Path) -> Vec<[f64; 3]> {
        stl::read_binary(path).unwrap().0.into_iter().flatten().map(|c| c.map(f64::from)).collect()
    }

    fn radii(path: &Path) -> Vec<f64> {
        let mut radii: Vec<f64> = corners(path).iter().map(|c| c.iter().map(|v| v * v).sum::<f64>().sqrt()).collect();
        radii.sort_by(f64::total_cmp);
        radii
    }

    #[test]
    fn observed_sphere_is_closed_and_accurate() {
        let scratch = Scratch::new("observed");
        sphere_volume(&scratch.0.join("v.npz"), true, None);
        let report = run(&scratch.0.join("v.npz"), &scratch.0.join("mesh"), &no_taubin(), 1, &EventLog::none(), None, 2).unwrap();
        assert!(report.closed);
        assert_eq!(report.genus, 0);
        assert!(report.signed_volume > 0.0);
        assert!(!report.flat_base_applied && report.hull_fraction_below_support.is_none());
        assert_eq!((report.components, report.discarded_faces), (1, 0));
        let radii = radii(&scratch.0.join("mesh/mesh.stl"));
        assert_eq!(radii.len(), report.triangles * 3);
        assert!((radii[radii.len() / 2] - 1.0).abs() < 0.03, "median radius {}", radii[radii.len() / 2]); // voxel is 0.05
        assert!(radii[radii.len() - 1] - radii[0] < 0.08, "radius spread {}", radii[radii.len() - 1] - radii[0]);
        // Normals stored in the file point away from the centre.
        let (triangles, normals) = stl::read_binary(&scratch.0.join("mesh/mesh.stl")).unwrap();
        assert!(triangles.iter().zip(&normals).all(|(t, n)| (0..3).map(|a| t[0][a] * n[a]).sum::<f32>() > 0.0));
        let written: serde_json::Value =
            serde_json::from_str(&crate::storage::read_to_string(scratch.0.join("mesh/result.json")).unwrap()).unwrap();
        let keys: Vec<&str> = written.as_object().unwrap().keys().map(String::as_str).collect();
        let mut expected = vec![
            "flat_base_applied",
            "observed_hull_fraction",
            "extrapolated_hull_fraction",
            "hull_overshoot_voxels",
            "components",
            "discarded_faces",
            "vertices",
            "triangles",
            "boundary_edges",
            "nonmanifold_edges",
            "genus",
            "signed_volume",
            "closed",
            "configuration",
            "reference_used",
            "physical_scale_established",
            "seconds",
        ];
        expected.sort_unstable();
        assert_eq!(keys, expected);
        assert_eq!(written["reference_used"], false);
        assert_eq!(written["physical_scale_established"], false);
        assert_eq!(written["configuration"]["mesh_taubin_cycles"], 0);
    }

    #[test]
    fn unobserved_volume_falls_back_to_the_hull() {
        let scratch = Scratch::new("unobserved");
        sphere_volume(&scratch.0.join("v.npz"), false, None);
        let report = run(&scratch.0.join("v.npz"), &scratch.0.join("mesh"), &no_taubin(), 1, &EventLog::none(), None, 2).unwrap();
        assert!(report.closed);
        assert_eq!(report.observed_hull_fraction, 0.0);
        assert_eq!(report.extrapolated_hull_fraction, 0.0);
        let radii = radii(&scratch.0.join("mesh/mesh.stl"));
        assert!((radii[radii.len() / 2] - 1.15).abs() < 0.05, "median radius {}", radii[radii.len() / 2]);
    }

    #[test]
    fn a_stopped_stage_leaves_no_output() {
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;
        let scratch = Scratch::new("stopped");
        sphere_volume(&scratch.0.join("v.npz"), true, None);
        let flag = Arc::new(AtomicBool::new(true));
        let control = crate::control::Control::new(Some(flag), None, None, None).unwrap();
        let error =
            run_with(&scratch.0.join("v.npz"), &scratch.0.join("mesh"), &DenseConfig::default(), 1, &EventLog::none(), None, 2, &control)
                .unwrap_err();
        assert_eq!(error.downcast_ref::<crate::control::Stopped>(), Some(&crate::control::Stopped::Cancelled));
        assert!(!scratch.0.join("mesh").exists());
        // The same volume meshes when nothing stops it.
        run_with(
            &scratch.0.join("v.npz"),
            &scratch.0.join("mesh"),
            &DenseConfig::default(),
            1,
            &EventLog::none(),
            None,
            2,
            &crate::control::Control::none(),
        )
        .unwrap();
        assert!(scratch.0.join("mesh/mesh.stl").is_file());
    }

    #[test]
    fn refuses_existing_output() {
        let scratch = Scratch::new("existing");
        sphere_volume(&scratch.0.join("v.npz"), true, None);
        crate::storage::create_dir(scratch.0.join("mesh")).unwrap();
        let error =
            run(&scratch.0.join("v.npz"), &scratch.0.join("mesh"), &DenseConfig::default(), 1, &EventLog::none(), None, 2).unwrap_err();
        assert!(error.to_string().contains("already exists"), "{error}");
        assert!(std::fs::read_dir(scratch.0.join("mesh")).unwrap().next().is_none(), "nothing may be written into it");
    }

    #[test]
    fn flat_base_cuts_the_solid_at_the_support() {
        let scratch = Scratch::new("base");
        // "Down" is +z and the lowest measured level is z = 0.3; one voxel (0.05) is kept below it.
        sphere_volume(&scratch.0.join("v.npz"), true, Some(0.3));
        let report = run(&scratch.0.join("v.npz"), &scratch.0.join("cut"), &no_taubin(), 1, &EventLog::none(), None, 2).unwrap();
        assert!(report.closed && report.flat_base_applied);
        assert_eq!(report.genus, 0);
        // Share of the hull ball (radius 1.15) beyond z = 0.35: a cap of height 0.8.
        let cap = std::f64::consts::PI * 0.8 * 0.8 * (3.0 * 1.15 - 0.8) / 3.0 / (4.0 / 3.0 * std::f64::consts::PI * 1.15f64.powi(3));
        let below = report.hull_fraction_below_support.unwrap();
        assert!((below - cap).abs() < 0.01, "{below} against {cap}");
        let cut = corners(&scratch.0.join("cut/mesh.stl"));
        let highest = cut.iter().map(|c| c[2]).fold(f64::MIN, f64::max);
        let lowest = cut.iter().map(|c| c[2]).fold(f64::MAX, f64::min);
        assert!((highest - 0.35).abs() < 0.03, "cut at {highest}");
        assert!((lowest + 1.0).abs() < 0.03, "far side at {lowest}");
        // The cut is flat: a good share of the surface lies in that plane.
        let flat = cut.iter().filter(|c| (c[2] - highest).abs() < 0.01).count();
        assert!(flat * 10 > cut.len(), "{flat} of {} corners in the base plane", cut.len());
        let full_volume = 4.0 / 3.0 * std::f64::consts::PI;
        assert!(report.signed_volume < 0.9 * full_volume && report.signed_volume > 0.6 * full_volume);

        // Switched off, or without a measured level, the sphere stays whole.
        let off = settings::build(None, &["mesh_taubin_cycles=0".to_string(), "mesh_flat_base=false".to_string()]).unwrap();
        let whole = run(&scratch.0.join("v.npz"), &scratch.0.join("whole"), &off, 1, &EventLog::none(), None, 2).unwrap();
        assert!(!whole.flat_base_applied && whole.hull_fraction_below_support.is_none());
        assert!((whole.signed_volume - full_volume).abs() < 0.05 * full_volume);
        sphere_volume(&scratch.0.join("nan.npz"), true, Some(f32::NAN));
        let unmeasured = run(&scratch.0.join("nan.npz"), &scratch.0.join("nan"), &no_taubin(), 1, &EventLog::none(), None, 2).unwrap();
        assert!(!unmeasured.flat_base_applied);
        assert_eq!(unmeasured.triangles, whole.triangles);
    }

    #[test]
    fn default_run_smooths_and_previews_do_not() {
        let scratch = Scratch::new("events");
        sphere_volume(&scratch.0.join("v.npz"), true, None);
        let log_path = crate::storage::canonicalize(&scratch.0).unwrap().join("events.jsonl");
        let log = EventLog::new(Some(&log_path), "mesh");
        let full = run(&scratch.0.join("v.npz"), &scratch.0.join("mesh"), &DenseConfig::default(), 1, &log, None, 1).unwrap();
        let preview_log = EventLog::new(Some(&log_path), "stereo");
        let preview = run(
            &scratch.0.join("v.npz"),
            &scratch.0.join("stereo/preview/hull"),
            &DenseConfig::default(),
            2,
            &preview_log,
            Some("Hull"),
            1,
        )
        .unwrap();
        assert!(full.closed && preview.closed);
        assert_eq!((full.genus, preview.genus), (0, 0));
        assert!(preview.triangles * 3 < full.triangles, "step 2 gives about a quarter of the triangles");
        // Five Taubin cycles keep the sphere a sphere.
        let radii = radii(&scratch.0.join("mesh/mesh.stl"));
        assert!((radii[radii.len() / 2] - 1.0).abs() < 0.03 && radii[radii.len() - 1] - radii[0] < 0.08);
        let events: Vec<serde_json::Value> =
            crate::storage::read_to_string(&log_path).unwrap().lines().map(|line| serde_json::from_str(line).unwrap()).collect();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["type"], "artifact");
        assert_eq!((events[0]["stage"].as_str(), events[0]["kind"].as_str()), (Some("mesh"), Some("final_mesh")));
        assert_eq!((events[0]["path"].as_str(), events[0]["label"].as_str()), (Some("mesh/mesh.stl"), Some("Final surface")));
        assert_eq!(events[0]["triangles"], full.triangles);
        assert_eq!((events[1]["stage"].as_str(), events[1]["kind"].as_str()), (Some("stereo"), Some("preview_mesh")));
        assert_eq!((events[1]["path"].as_str(), events[1]["label"].as_str()), (Some("stereo/preview/hull/mesh.stl"), Some("Hull")));
    }

    #[test]
    fn command_line_is_parsed_like_the_reference() {
        let words = |text: &str| text.split(' ').map(String::from).collect::<Vec<_>>();
        let parsed =
            parse(&words("--volume v.npz --output out --set mesh_smooth=2 --set=grid=320 --step 2 --threads=3 --label Hull")).unwrap();
        assert_eq!((parsed.volume, parsed.output), (PathBuf::from("v.npz"), PathBuf::from("out")));
        assert_eq!(parsed.set, vec!["mesh_smooth=2", "grid=320"]);
        assert_eq!((parsed.step, parsed.threads, parsed.label.as_deref()), (2, 3, Some("Hull")));
        assert!(parsed.config.is_none() && parsed.events.is_none());
        let defaults = parse(&words("--volume v.npz --output out")).unwrap();
        assert_eq!((defaults.step, defaults.threads), (1, 2));
        for bad in
            ["--output out", "--volume v.npz", "--volume v.npz --output out --step 0", "--volume v.npz --output out --fast", "--volume"]
        {
            assert!(parse(&words(bad)).is_err(), "{bad:?} should be refused");
        }
    }
}
