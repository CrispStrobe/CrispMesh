//! The stage driver: port of `run` in `multiscale_stereo.py`.

use std::path::Path;
use web_time::Instant;

use anyhow::{bail, Context};
use serde_json::{json, Value};

use crate::config::DenseConfig;
use crate::control::Control;
use crate::events::EventLog;
use crate::fusion::{tsdf, Fused, Support};
use crate::gpu::Gpu;
use crate::hull::{build_hull, Hull, HullState, VoxelList};
use crate::inputs::{parallel_map, round_half_even, Inputs, Plane};
use crate::npz::{self, Array, Data, Npz};
use crate::repair::{repair_masks_with, Coverer};

use super::level::{build_level, level_sizes};
use super::levels::{match_levels, LevelContext};
use super::previews::colour_sheet;
use super::Arguments;

/// `index` rows (x, y, z) of the listed voxels as an N x 3 int32 array.
pub fn index_array(hull: &Hull, indices: &[u32]) -> Array {
    let mut data = Vec::with_capacity(indices.len() * 3);
    for &linear in indices {
        let [i, j, k] = hull.unravel(linear);
        data.extend_from_slice(&[i as i32, j as i32, k as i32]);
    }
    Array::new(&[indices.len(), 3], Data::I32(data))
}

/// Writes a volume file with the reference's member names, types and shapes.
#[allow(clippy::too_many_arguments)]
pub fn write_volume(
    path: &Path,
    compress: bool,
    hull: &Hull,
    indices: &[u32],
    total: &[f32],
    weight: &[f32],
    truncation: f64,
    support: Option<&Support>,
) -> anyhow::Result<()> {
    write_volume_with(path, compress, hull, indices, total, weight, truncation, support, false, false)
}

/// [`write_volume`] with the `partial_views` and `silhouette_first` members (each written only when true).
#[allow(clippy::too_many_arguments)]
pub fn write_volume_with(
    path: &Path,
    compress: bool,
    hull: &Hull,
    indices: &[u32],
    total: &[f32],
    weight: &[f32],
    truncation: f64,
    support: Option<&Support>,
    partial_views: bool,
    silhouette_first: bool,
) -> anyhow::Result<()> {
    let f32s = |values: &[f32]| Array::new(&[values.len()], Data::F32(values.to_vec()));
    let index = index_array(hull, indices);
    let (total, weight) = (f32s(total), f32s(weight));
    let shape = Array::new(&[3], Data::I64(hull.shape.iter().map(|&s| s as i64).collect()));
    let origin = f32s(&hull.origin);
    let voxel = Array::scalar_f32(hull.voxel as f32);
    let truncation = Array::scalar_f32(truncation as f32);
    let mut arrays: Vec<(&str, &Array)> = vec![
        ("index", &index),
        ("total", &total),
        ("weight", &weight),
        ("shape", &shape),
        ("origin", &origin),
        ("voxel", &voxel),
        ("truncation", &truncation),
    ];
    let extra = support.map(|s| (f32s(&s.point), f32s(&s.down), Array::scalar_f32(s.height.unwrap_or(f32::NAN))));
    if let Some((point, down, height)) = &extra {
        arrays.extend([("support_point", point), ("support_down", down), ("support_height", height)]);
    }
    let partial = Array::scalar_f32(1.0);
    if partial_views {
        arrays.push(("partial_views", &partial));
    }
    if silhouette_first {
        arrays.push(("silhouette_first", &partial));
    }
    npz::write(path, &arrays, compress)
}

/// `depths.npz`: one `depth_NNN` float32 array (height x width) per view.
pub fn write_depths(path: &Path, depths: &[Plane<f32>]) -> anyhow::Result<()> {
    let arrays: Vec<(String, Array)> = depths
        .iter()
        .enumerate()
        .map(|(i, d)| (format!("depth_{i:03}"), Array::new(&[d.height, d.width], Data::F32(d.data.clone()))))
        .collect();
    let members: Vec<(&str, &Array)> = arrays.iter().map(|(name, array)| (name.as_str(), array)).collect();
    npz::write(path, &members, true)
}

pub fn read_depths(path: &Path, count: usize) -> anyhow::Result<Vec<Plane<f32>>> {
    let archive = Npz::read(path)?;
    (0..count)
        .map(|i| {
            let array = archive.get(&format!("depth_{i:03}"))?;
            if array.shape.len() != 2 {
                bail!("depth_{i:03} is not an image");
            }
            Ok(Plane { width: array.shape[1], height: array.shape[0], data: array.to_f32() })
        })
        .collect()
}

/// Volumes the driver meshes coarsely while matching continues, renamed into place when complete.
pub struct Previews<'a> {
    directory: Option<std::path::PathBuf>,
    events: &'a EventLog,
    config: &'a DenseConfig,
    /// Hand the volumes to the preview mesher in memory instead of writing them.
    hand_over: bool,
}

impl Previews<'_> {
    pub fn enabled(&self) -> bool {
        self.directory.is_some()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn write(
        &self,
        name: &str,
        label: &str,
        hull: &Hull,
        indices: &[u32],
        total: &[f32],
        weight: &[f32],
        support: Option<&Support>,
        meta: Value,
    ) -> anyhow::Result<()> {
        let Some(directory) = &self.directory else { return Ok(()) };
        let (partial, target) = (directory.join(format!("{name}.partial")), directory.join(format!("{name}.npz")));
        let truncation = self.config.truncation_voxels * hull.voxel;
        if self.hand_over {
            let volume = crate::mesh::field::Volume::from_parts(hull, indices, total.to_vec(), weight.to_vec(), truncation, support);
            crate::mesh::field::Volume::hand_over(&target, volume);
        } else {
            write_volume(&partial, false, hull, indices, total, weight, truncation, support)?;
            crate::storage::rename(&partial, &target)?;
        }
        self.events.artifact("preview_volume", &target, label, meta)
    }

    fn hull(&self, name: &str, label: &str, hull: &Hull) -> anyhow::Result<()> {
        if self.directory.is_none() {
            return Ok(());
        }
        let indices = hull.indices();
        let zeros = vec![0.0f32; indices.len()];
        self.write(name, label, hull, &indices, &zeros, &zeros, None, json!({}))
    }
}

fn hull_report(state: &HullState, since: Instant) -> anyhow::Result<Value> {
    let mut report = serde_json::to_value(&state.report)?;
    report["seconds"] = json!(since.elapsed().as_secs_f64());
    Ok(report)
}

/// The stage as a command: its own device, events appended to `arguments.events`, log on standard output.
#[cfg(not(target_arch = "wasm32"))]
pub fn run(arguments: &Arguments, config: &DenseConfig) -> anyhow::Result<Value> {
    let events = EventLog::new(arguments.events.as_deref(), "stereo");
    crate::gpu::block_on(run_with(arguments, config, &Gpu::new()?, &events, &Control::none()))
}

/// The stage inside a larger run: on the caller's device, reporting to the
/// caller's event log, stopping when `control` says so (checked once per view
/// while matching and between the other steps).
pub async fn run_with(
    arguments: &Arguments,
    config: &DenseConfig,
    gpu: &Gpu,
    events: &EventLog,
    control: &Control,
) -> anyhow::Result<Value> {
    run_fused(arguments, config, gpu, events, control, Files::all()).await.map(|(report, _)| report)
}

/// Which files the stage writes at its end. Inside one `run` the fused volume
/// goes to the surface stage in memory ([`run_fused`]), so `volume.npz` is
/// written only when it is kept, and `depths.npz` only where a later run can
/// reuse it (not in a browser).
#[derive(Debug, Clone, Copy)]
pub struct Files {
    pub volume: bool,
    pub depths: bool,
    /// Preview volumes go to the preview mesher of the same run in memory
    /// (`mesh::field::Volume::hand_over`); their `preview_volume` events name
    /// a path where no file is written.
    pub preview_volumes_in_memory: bool,
}

impl Files {
    /// All files, as the `stereo` command and `run_with` do.
    pub fn all() -> Self {
        Files { volume: true, depths: true, preview_volumes_in_memory: false }
    }
}

/// `mask_grow`: a first pass over all but the finest level. Depths that views from other directions
/// confirm are surface points; where they land outside a mask, the mask left out part of the object
/// (pale parts on a pale backdrop). The grown masks are written to `masks-grown`; the run then starts
/// again from them, because a hull carved from masks that cut the object bounds the depth search wrongly.
async fn grow_masks(
    arguments: &Arguments,
    config: &DenseConfig,
    gpu: &Gpu,
    events: &EventLog,
    control: &Control,
    inputs: &mut Inputs,
) -> anyhow::Result<Value> {
    let t = Instant::now();
    let output = &arguments.output;
    let scratch = output.join("mask-grow");
    crate::storage::create_dir(&scratch)?;
    let mut voxel = None;
    let mut state = build_hull(gpu, inputs, config, &mut voxel).await?;
    if config.repair_masks {
        for _ in 0..config.repair_rounds {
            control.check()?;
            repair_masks_with(gpu, inputs, &state, config, &|| control.check()).await?;
            state = build_hull(gpu, inputs, config, &mut voxel).await?;
        }
    }
    control.log(format!("mask grow: hull {} voxels", state.hull.indices().len()));
    inputs.crop_gray();
    let sizes = crate::stereo::level::with_native_level(level_sizes(&config.sizes, inputs.longest), inputs.longest, config.native_level);
    // All but the finest level; with two levels or fewer, all of them (one coarse level alone says little).
    let coarse = if sizes.len() > 2 { &sizes[..sizes.len() - 1] } else { &sizes[..] };
    control.log(format!("mask grow: levels {coarse:?} of {sizes:?}"));
    let count = inputs.count();
    let picks = [count / 7, (count / 2).saturating_sub(3), (4 * count) / 5];
    let previews = Previews { directory: None, events, config, hand_over: false };
    let mut report = json!({"levels": []});
    let (level, depths) = {
        let context =
            LevelContext { gpu, inputs, state: &state, config, output: &scratch, events, previews: &previews, picks: &picks, control };
        match_levels(&context, coarse, &mut report).await?
    };
    control.log(format!("mask grow: matched {} views", depths.len()));
    control.check()?;
    let (checked, kept) = crate::fusion::cross_checked(
        &level,
        &depths,
        &inputs.angles,
        (20.0, 80.0),
        16,
        config.cross_check_tolerance as f32,
        config.cross_check_votes as usize,
    );
    control.log(format!("mask grow: confirmed {kept:.3}"));
    let (grown, added) = crate::fusion::grown_masks(&level, &checked, &inputs.cameras, &inputs.masks, 3, config.mask_grow_close as usize);
    crate::storage::create_dir(output.join("masks-grown"))?;
    let written: Vec<anyhow::Result<()>> = parallel_map(count, |n| {
        let pixels: Vec<u8> = grown[n].data.iter().map(|&m| m * 255).collect();
        crate::storage::save_png(
            output.join("masks-grown").join(format!("{}.png", inputs.rows[n].name)),
            grown[n].width,
            grown[n].height,
            1,
            &pixels,
        )
    });
    written.into_iter().collect::<anyhow::Result<()>>()?;
    Ok(json!({
        "level_size": coarse.last(), "confirmed_fraction": kept, "added_fraction_median": added, "seconds": t.elapsed().as_secs_f64(),
    }))
}

/// [`run_with`] that also returns the fused volume, in the form `volume.npz`
/// would give it back, and writes only the files `files` asks for. Depth maps
/// are released as soon as fusion and their file are done.
pub async fn run_fused(
    arguments: &Arguments,
    config: &DenseConfig,
    gpu: &Gpu,
    events: &EventLog,
    control: &Control,
    files: Files,
) -> anyhow::Result<(Value, Option<crate::mesh::field::Volume>)> {
    let output = &arguments.output;
    if crate::storage::exists(output) {
        bail!("output directory exists: {}", output.display());
    }
    crate::storage::create_dir_all(output).with_context(|| output.display().to_string())?;
    let started = Instant::now();
    let mut report = json!({
        "configuration": serde_json::to_value(config)?,
        "device": format!("wgpu: {}", gpu.describe()),
        "engine": concat!("crispmesh-dense ", env!("CARGO_PKG_VERSION")),
        "levels": [],
        "reuse_depths": arguments.reuse_depths.as_ref().map(|p| p.display().to_string()),
    });
    let t = Instant::now();
    let mut inputs = if arguments.only.as_deref() == Some("hull") {
        Inputs::load_hull(&arguments.inputs, config)?
    } else {
        Inputs::load(&arguments.inputs, config)?
    };
    // Silhouette first: asked for, or left to the capture (cameras at several heights).
    let silhouette_first =
        config.silhouette_first == 1 || (config.silhouette_first == 2 && crate::fusion::several_heights(&inputs.cameras));
    report["silhouette_first"] = json!(silhouette_first);
    let chosen;
    let config = if silhouette_first {
        let mut adjusted = config.clone();
        adjusted.cross_check = true;
        adjusted.mask_grow = true;
        chosen = adjusted;
        control.log("silhouette first: cameras at several heights, or asked for");
        &chosen
    } else {
        config
    };
    if config.mask_grow {
        // The first pass's GPU buffers are freed before the run proper allocates its own.
        gpu.track();
        let grown = grow_masks(arguments, config, gpu, events, control, &mut inputs).await;
        gpu.release_tracked();
        report["mask_grow"] = grown?;
        control.log(format!("mask grow {}", report["mask_grow"]));
        inputs = Inputs::load_with_masks(&arguments.inputs, config, Some(&output.join("masks-grown")))?;
    }
    let count = inputs.count();
    report["views"] = json!(count);
    report["load_seconds"] = json!(t.elapsed().as_secs_f64());
    let picks = [count / 7, (count / 2).saturating_sub(3), (4 * count) / 5];
    let previews = Previews {
        directory: arguments.previews.then(|| output.join("preview")),
        events,
        config,
        hand_over: files.preview_volumes_in_memory,
    };
    if let Some(directory) = &previews.directory {
        crate::storage::create_dir(directory)?;
    }

    let t = Instant::now();
    let mut voxel = None;
    let mut state = build_hull(gpu, &inputs, config, &mut voxel).await?;
    report["hull"] = hull_report(&state, t)?;
    control.log(format!("hull {}", report["hull"]));
    events.progress(0.03, "Silhouette hull built")?;
    control.check()?;
    previews.hull("00-hull", "Silhouette hull", &state.hull)?;

    if arguments.only.as_deref() == Some("hull") {
        // Diagnostic: the hull and what `Stereo.__init__` derived, for parity checks.
        let indices = state.hull.indices();
        let zeros = vec![0.0f32; indices.len()];
        let truncation = config.truncation_voxels * state.hull.voxel;
        write_volume(&output.join("volume.npz"), true, &state.hull, &indices, &zeros, &zeros, truncation, None)?;
        report["bounds"] = json!(state.bounds);
        report["boxes"] = json!(inputs.boxes);
        report["longest"] = json!(inputs.longest);
        report["repair_loose"] = json!(inputs.repair_loose);
        report["gray_crc32"] = json!(inputs.gray.iter().map(|g| crc32fast::hash(bytemuck::cast_slice(&g.data))).collect::<Vec<_>>());
        // Diagnostic only. Sparse/irregular captures can carve a hull without
        // the neighbours needed for stereo; retain its stricter gate below.
        let neighbours: Vec<Option<Vec<usize>>> =
            (0..count).map(|i| inputs.neighbours(i, config.vote_neighbours as usize, config).ok()).collect();
        report["neighbours"] = json!(neighbours);
        return finish(report, output, started).map(|report| (report, None));
    }

    if config.repair_masks {
        let t = Instant::now();
        let before: Vec<Plane<u8>> = picks.iter().map(|&i| inputs.masks[i].clone()).collect();
        let mut rounds = Vec::new();
        for _ in 0..config.repair_rounds {
            control.check()?;
            rounds.push(repair_masks_with(gpu, &mut inputs, &state, config, &|| control.check()).await?);
            control.check()?;
            state = build_hull(gpu, &inputs, config, &mut voxel).await?;
            report["hull_repaired"] = hull_report(&state, t)?;
        }
        let added: f64 = rounds.iter().map(|r| r.added_fraction_median).sum();
        report["mask_repair"] = json!({"loose_views": inputs.repair_loose, "rounds": rounds, "added_fraction_median": added});
        control.log(format!("repair {} {}", report["mask_repair"], report["hull_repaired"]));
        crate::storage::create_dir(output.join("masks-repaired"))?;
        let written: Vec<anyhow::Result<()>> = parallel_map(count, |n| {
            let mask = &inputs.masks[n];
            let pixels: Vec<u8> = mask.data.iter().map(|&m| m * 255).collect();
            let path = output.join("masks-repaired").join(format!("{}.png", inputs.rows[n].name));
            crate::storage::save_png(&path, mask.width, mask.height, 1, &pixels)
        });
        written.into_iter().collect::<anyhow::Result<()>>()?;
        let sheet = output.join("mask-repair.png");
        colour_sheet(&sheet, &inputs, &picks, |i| {
            let old = &before[picks.iter().position(|&p| p == i).unwrap()];
            let new = &inputs.masks[i];
            let added = new.data.iter().zip(&old.data).map(|(&n, &o)| (n != 0 && o == 0) as u8).collect();
            vec![(Plane { width: new.width, height: new.height, data: added }, [0.1, 0.9, 0.2])]
        })?;
        events.artifact("mask_repair_sheet", &sheet, "Mask repair (added pixels in green)", json!({}))?;
        events.metric("mask_added_fraction_median", added)?;
        previews.hull("01-hull-repaired", "Silhouette hull after mask repair", &state.hull)?;
    }
    if arguments.only.as_deref() == Some("repair") {
        return finish(report, output, started).map(|report| (report, None));
    }

    {
        let list = VoxelList::new(gpu, &state.hull, &state.hull.indices());
        let coverer = Coverer::new(gpu).await?;
        let mut covers = Vec::new();
        for &i in &picks {
            covers.push(coverer.cover(&list, &inputs.cameras[i], inputs.masks[i].width, inputs.masks[i].height).await?);
        }
        let sheet = output.join("hull-vs-mask.png");
        colour_sheet(&sheet, &inputs, &picks, |i| {
            let cover = &covers[picks.iter().position(|&p| p == i).unwrap()];
            let mask = &inputs.masks[i];
            let plane = |data: Vec<u8>| Plane { width: mask.width, height: mask.height, data };
            let uncovered = mask.data.iter().zip(&cover.data).map(|(&m, &c)| (m != 0 && c == 0) as u8).collect();
            let outside = mask.data.iter().zip(&cover.data).map(|(&m, &c)| (m == 0 && c != 0) as u8).collect();
            vec![(plane(uncovered), [1.0, 0.1, 0.1]), (plane(outside), [0.1, 0.5, 1.0])]
        })?;
        events.artifact("hull_mask_sheet", &sheet, "Hull against masks (red: mask not covered, blue: hull outside mask)", json!({}))?;
    }
    events.progress(0.08, "Masks and hull ready")?;
    control.check()?;
    // From here on only the canvas part of each photo is read.
    inputs.crop_gray();

    let sizes = crate::stereo::level::with_native_level(level_sizes(&config.sizes, inputs.longest), inputs.longest, config.native_level);
    if arguments.only.as_deref() == Some("levels") {
        // Diagnostic: checksums of every level image, mask and camera for parity checks against Pillow.
        let mut levels = Vec::new();
        for &size in &sizes {
            let level = build_level(&inputs, size);
            levels.push(json!({
                "size": size,
                "working_size": [level[0].width, level[0].height],
                "gray_crc32": level.iter().map(|v| crc32fast::hash(bytemuck::cast_slice(&v.gray.data))).collect::<Vec<_>>(),
                "mask_crc32": level.iter().map(|v| crc32fast::hash(&v.mask.data)).collect::<Vec<_>>(),
                "k": level.iter().map(|v| v.camera.k.map(|x| x as f64)).collect::<Vec<_>>(),
            }));
        }
        report["level_checks"] = json!(levels);
        return finish(report, output, started).map(|report| (report, None));
    }
    let (mut level, mut depths) = match &arguments.reuse_depths {
        Some(path) => (build_level(&inputs, *sizes.last().unwrap()), read_depths(path, count)?),
        None => {
            if config.fused_passes != 0 {
                bail!("fused_passes is not ported to the native stage; use fused_passes=0");
            }
            let context =
                LevelContext { gpu, inputs: &inputs, state: &state, config, output, events, previews: &previews, picks: &picks, control };
            match_levels(&context, &sizes, &mut report).await?
        }
    };

    // Depths confirmed across directions.
    if config.cross_check {
        let t = Instant::now();
        let (checked, kept) = crate::fusion::cross_checked(
            &level,
            &depths,
            &inputs.angles,
            (20.0, 80.0),
            16,
            config.cross_check_tolerance as f32,
            config.cross_check_votes as usize,
        );
        depths = checked;
        report["cross_check"] = json!({"kept_fraction": kept, "seconds": t.elapsed().as_secs_f64()});
        control.log(format!("cross check {}", report["cross_check"]));
        events.metric("cross_check_kept_fraction", kept)?;
        control.check()?;
    }
    // Fusion reads masks, cameras and depth; the grey images are done.
    inputs.gray = Vec::new();
    for view in &mut level {
        view.gray = Plane::new(0, 0);
    }
    let t = Instant::now();
    let rim = round_half_even(config.rim_fraction * DenseConfig::level(&config.windows, sizes.len() - 1) as f64) as usize;
    // Noise-matched truncation: depths that disagree between views by more than the
    // fixed band would be clipped, not averaged; widen the band to the measured noise.
    let widened;
    let config = if config.truncation_noise > 0.0 {
        let voters: Vec<Vec<usize>> =
            (0..count).map(|i| inputs.neighbours(i, config.vote_neighbours as usize, config)).collect::<anyhow::Result<_>>()?;
        let mut adjusted = config.clone();
        match crate::fusion::depth_noise(&level, &depths, &voters, 2, 0.02) {
            Some((sigma, samples)) => {
                let noise_voxels = sigma / state.hull.voxel;
                adjusted.truncation_voxels = config.truncation_voxels.max(config.truncation_noise * noise_voxels);
                adjusted.behind_voxels = config.behind_voxels.max(adjusted.truncation_voxels);
                report["depth_noise"] = json!({
                    "sigma": sigma, "sigma_voxels": noise_voxels, "samples": samples,
                    "truncation_voxels": adjusted.truncation_voxels, "behind_voxels": adjusted.behind_voxels,
                });
            }
            None => report["depth_noise"] = json!({"sigma": null, "truncation_voxels": config.truncation_voxels}),
        }
        control.log(format!("depth noise {}", report["depth_noise"]));
        widened = adjusted;
        &widened
    } else {
        config
    };
    let mut fused: Fused = tsdf(gpu, &state.hull, &inputs.cameras, &level, &depths, rim, config).await?;
    fused.silhouette_first = silhouette_first;
    if config.support_from_sparse {
        if let Some((measured, sparse)) = crate::fusion::support_from_sparse(&mut fused.support, &inputs.sparse) {
            report["support"] = json!({"measured_level": measured, "sparse_level": sparse, "raised_voxels": ((measured - sparse) / state.hull.voxel as f32).max(0.0)});
        }
    }
    let evidence = crate::fusion::support_evidence(&state.hull, &inputs.cameras, &fused.support);
    report["support_evidence"] = json!({
        "support": fused.support.height.is_some(), "hull_depth_below_voxels": evidence.depth, "height_voxels": evidence.height,
        "cone_voxels": evidence.cone, "elevation_degrees": evidence.elevation,
    });
    report["rim_pixels"] = json!(rim);
    report["fused_passes"] = json!([]);
    let observed = fused.weight.iter().filter(|&&w| w > 0.0).count() as f64 / fused.weight.len().max(1) as f64;
    report["tsdf"] = json!({
        "hull_voxels": fused.indices.len(), "observed_fraction": observed,
        "weight_kind": "signed-distance vote mass including distant occupancy",
        "seconds": t.elapsed().as_secs_f64(),
    });
    control.log(format!("tsdf {}", report["tsdf"]));
    events.progress(0.97, "Depth fused")?;

    let t = Instant::now();
    if files.depths {
        write_depths(&output.join("depths.npz"), &depths)?;
    }
    drop(depths);
    drop(level);
    if files.volume {
        write_volume_with(
            &output.join("volume.npz"),
            true,
            &state.hull,
            &fused.indices,
            &fused.total,
            &fused.weight,
            fused.truncation,
            Some(&fused.support),
            fused.partial_views,
            fused.silhouette_first,
        )?;
    }
    report["partial_views"] = json!(fused.partial_views);
    report["write_seconds"] = json!(t.elapsed().as_secs_f64());
    let volume = crate::mesh::field::Volume::from_fused(&state.hull, fused);
    finish(report, output, started).map(|report| (report, Some(volume)))
}

fn finish(mut report: Value, output: &Path, started: Instant) -> anyhow::Result<Value> {
    report["seconds"] = json!(started.elapsed().as_secs_f64());
    report["reference_used"] = json!(false);
    crate::storage::write(output.join("result.json"), serde_json::to_string_pretty(&report)? + "\n")?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stereo::{options, synthetic};

    /// The whole stage on the analytic sphere, with the thresholds of
    /// scripts/turntable_mesh/test_multiscale_stereo.py (CRISPMESH_GPU_TESTS=1).
    #[test]
    fn gpu_stage_recovers_the_sphere() {
        if !crate::gpu::tests_enabled() {
            return;
        }
        let root = synthetic::temporary("stage", 24, 128).unwrap();
        let overrides: Vec<String> = synthetic::SMALL.iter().map(|s| s.to_string()).collect();
        let config = options::build(None, &overrides).unwrap();
        let arguments = Arguments {
            inputs: root.join("inputs"),
            output: root.join("stereo"),
            events: Some(root.join("events.jsonl")),
            previews: true,
            ..Default::default()
        };
        let report = run(&arguments, &config).unwrap();
        assert_eq!(report["views"], 24);
        assert_eq!(report["reference_used"], false);
        assert_eq!(report["levels"].as_array().unwrap().len(), 3);
        assert!(report["gpu_peak_binding_bytes"].as_u64().unwrap() <= 128 << 20);

        // Depth against the exact sphere depth, cropped as the stage crops.
        let exact = Npz::read(&root.join("inputs/exact_depths.npz")).unwrap();
        let found = read_depths(&root.join("stereo/depths.npz"), 24).unwrap();
        let pad = config.crop_padding as usize;
        let (mut errors, mut covered) = (Vec::new(), Vec::new());
        for (n, depth) in found.iter().enumerate() {
            let truth = exact.get(&format!("view_{n:03}")).unwrap();
            let (th, tw) = (truth.shape[0], truth.shape[1]);
            let values = truth.to_f32();
            let hit: Vec<(usize, usize)> = (0..th * tw).filter(|&p| values[p] != 0.0).map(|p| (p % tw, p / tw)).collect();
            let (x0, x1) = (hit.iter().map(|h| h.0).min().unwrap() - pad, hit.iter().map(|h| h.0).max().unwrap() + pad + 1);
            let (y0, y1) = (hit.iter().map(|h| h.1).min().unwrap() - pad, hit.iter().map(|h| h.1).max().unwrap() + pad + 1);
            // The common canvas is centred on the mask box; even sizes may add one pixel.
            let (ox, oy) = ((depth.width - (x1 - x0)) / 2, (depth.height - (y1 - y0)) / 2);
            let (mut both, mut inside) = (0usize, 0usize);
            for y in y0..y1 {
                for x in x0..x1 {
                    let t = values[y * tw + x];
                    let d = depth.data[(y - y0 + oy) * depth.width + (x - x0 + ox)];
                    inside += (t > 0.0) as usize;
                    if t > 0.0 && d > 0.0 {
                        both += 1;
                        errors.push(((d - t).abs() / t) as f64);
                    }
                }
            }
            covered.push(both as f64 / inside as f64);
        }
        errors.sort_by(f64::total_cmp);
        let median_error = errors[errors.len() / 2];
        let p90_error = errors[(errors.len() as f64 * 0.9) as usize];
        let median_covered = crate::inputs::median_f64(&mut covered);
        assert!(median_covered > 0.6, "coverage {median_covered}");
        assert!(median_error < 0.003, "median error {median_error}");
        assert!(p90_error < 0.01, "p90 error {p90_error}");

        // Volume and report, as test_volume_and_report checks them.
        let volume = Npz::read(&root.join("stereo/volume.npz")).unwrap();
        let (index, total, weight) = (volume.get("index").unwrap(), volume.get("total").unwrap(), volume.get("weight").unwrap());
        assert_eq!(index.shape[0], total.len());
        let observed = weight.to_f32().iter().filter(|&&w| w > 0.0).count() as f64 / weight.len() as f64;
        assert!(observed > 0.3, "{observed}");
        let hull_volume = index.shape[0] as f64 * volume.get("voxel").unwrap().scalar().unwrap().powi(3);
        assert!(hull_volume > 4.0 && hull_volume < 7.0, "{hull_volume}");
        assert!(root.join("stereo/masks-repaired/view_000.png").is_file());
        for file in ["depth-level-0.png", "depth-level-1.png", "depth-merged.png", "hull-vs-mask.png", "mask-repair.png", "result.json"] {
            assert!(root.join("stereo").join(file).is_file(), "{file}");
        }
        // Events: progress, sheets and preview volumes whose files exist.
        let log = crate::storage::read_to_string(root.join("events.jsonl")).unwrap();
        let lines: Vec<Value> = log.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert!(lines.iter().all(|l| l["stage"] == "stereo"));
        let kinds: Vec<&str> = lines.iter().filter(|l| l["type"] == "artifact").map(|l| l["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds.iter().filter(|k| **k == "preview_volume").count(), 3);
        assert_eq!(kinds.iter().filter(|k| **k == "depth_sheet").count(), 3);
        for line in lines.iter().filter(|l| l["type"] == "artifact") {
            assert!(root.join(line["path"].as_str().unwrap()).is_file(), "{line}");
        }
        assert!(lines.iter().any(|l| l["type"] == "metric" && l["name"] == "coverage_level_1"));
        // A second run into the same directory is refused.
        assert!(run(&arguments, &config).is_err());
        crate::storage::remove_dir_all(&root).unwrap();
    }
}
