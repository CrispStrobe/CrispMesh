//! Photo check of a reconstructed STL: port of
//! `scripts/turntable_mesh/mesh_photo_check.py`.
//!
//! Reports silhouette intersection-over-union of the projected mesh against
//! each view's mask (the input masks and, when given, the repaired ones), and
//! writes an overlay sheet and a preview sheet (photos above, shaded mesh
//! below) for a few evenly spaced views. This is photo agreement: it cannot see
//! errors along the viewing direction and does not certify scanner accuracy or
//! physical scale.

#![allow(clippy::needless_range_loop)]

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail};
use serde_json::{json, Value};

use crate::events::EventLog;
use crate::inputs::{load_views, median_f64, parallel_map, round_half_even, ViewRow};
use crate::render::{fill_triangles, shade, with_thousands, Rgb, ViewCamera};
use crate::scene::read_gray;
use crate::stl::Triangle;

pub struct Options<'a> {
    pub inputs: &'a Path,
    pub mesh: &'a Path,
    pub output: &'a Path,
    pub repaired_masks: Option<&'a Path>,
    pub preview_views: usize,
    pub check_views: usize,
}

fn camera(row: &ViewRow) -> ViewCamera {
    ViewCamera { rotation: row.rotation, translation: row.translation, k: row.k }
}

/// The mesh silhouette in one view, as the reference draws it: corners at
/// pixel-index coordinates rounded to sixteenths, filled like `cv2.fillPoly`.
fn silhouette(triangles: &[Triangle], row: &ViewRow, width: usize, height: usize) -> anyhow::Result<Vec<u8>> {
    let view = camera(row);
    let mut points = Vec::with_capacity(triangles.len());
    for triangle in triangles {
        let mut corners = [[0i64; 2]; 3];
        for (corner, p) in corners.iter_mut().zip(triangle) {
            let cam = view.to_camera(p.map(|v| v as f64));
            if cam[2] <= 0.0 {
                bail!("mesh reaches behind camera {}", row.name);
            }
            let x = cam[0] / cam[2] * view.k[0] + (view.k[2] - 0.5);
            let y = cam[1] / cam[2] * view.k[1] + (view.k[3] - 0.5);
            *corner = [(x * 16.0).round_ties_even() as i64, (y * 16.0).round_ties_even() as i64];
        }
        points.push(corners);
    }
    Ok(fill_triangles(&points, width, height))
}

struct Scored {
    /// IoU against each mask set.
    scores: Vec<f64>,
    /// Overlay tile for preview views.
    tile: Option<Rgb>,
}

fn score_view(triangles: &[Triangle], row: &ViewRow, sets: &[Option<&Path>], overlay: bool) -> anyhow::Result<Scored> {
    let first = match sets[0] {
        Some(directory) => directory.join(format!("{}.png", row.name)),
        None => PathBuf::from(&row.mask),
    };
    let size = read_gray(&first).map_err(|_| anyhow!("missing or mismatched mask {}", first.display()))?;
    let (width, height) = (size.width, size.height);
    let drawn = silhouette(triangles, row, width, height)?;
    if let Ok(directory) = std::env::var("CRISPMESH_CHECK_DUMP") {
        // Diagnostic: the drawn silhouette of every checked view, for comparison with OpenCV.
        let path = Path::new(&directory).join(format!("{}.png", row.name));
        crate::storage::save_png(path, width, height, 1, &drawn)?;
    }
    let mut scores = Vec::new();
    let mut last = size;
    for (n, set) in sets.iter().enumerate() {
        if n > 0 {
            let path = set.map(|d| d.join(format!("{}.png", row.name))).unwrap_or_else(|| PathBuf::from(&row.mask));
            last = read_gray(&path)
                .ok()
                .filter(|m| (m.width, m.height) == (width, height))
                .ok_or_else(|| anyhow!("missing or mismatched mask {}", path.display()))?;
        }
        let (mut both, mut either) = (0usize, 0usize);
        for (&m, &d) in last.data.iter().zip(&drawn) {
            both += (m > 127 && d > 0) as usize;
            either += (m > 127 || d > 0) as usize;
        }
        scores.push(both as f64 / either.max(1) as f64);
    }
    let mut tile = None;
    if overlay {
        // The last mask set against the mesh outline: green both, red mask only, blue mesh only.
        let mut photo = Rgb::open(Path::new(&row.image))?;
        if (photo.width, photo.height) != (width, height) {
            bail!("photo and mask sizes differ for {}", row.name);
        }
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
        for p in 0..width * height {
            let (a, b) = (last.data[p] > 127, drawn[p] > 0);
            if !(a || b) {
                continue;
            }
            let colour: [f64; 3] = if a && b {
                [60.0, 170.0, 60.0]
            } else if a {
                [230.0, 40.0, 40.0]
            } else {
                [40.0, 120.0, 230.0]
            };
            let (x, y) = (p % width, p / width);
            let old = photo.pixel(x, y);
            photo.set(x, y, [0, 1, 2].map(|n| (0.45 * old[n] as f64 + 0.55 * colour[n]) as u8));
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
        if x0 == usize::MAX {
            bail!("neither mask nor mesh is visible in {}", row.name);
        }
        let crop = photo.crop(x0.saturating_sub(20), y0.saturating_sub(20), (x1 + 20).min(width), (y1 + 20).min(height));
        let tile_height = (round_half_even(600.0 * crop.height as f64 / crop.width as f64) as usize).max(1);
        tile = Some(crop.resize(600, tile_height));
    }
    Ok(Scored { scores, tile })
}

fn summary(names: &[&str], values: &[f64]) -> Value {
    let mut sorted = values.to_vec();
    let worst = (0..values.len()).min_by(|&a, &b| values[a].total_cmp(&values[b])).unwrap_or(0);
    json!({
        "median": median_f64(&mut sorted),
        "minimum": values.iter().cloned().fold(f64::MAX, f64::min),
        "maximum": values.iter().cloned().fold(f64::MIN, f64::max),
        "worst_view": names[worst],
    })
}

/// Preview sheet (`stl_compare_render.run` with one mesh): photos above, the shaded mesh below.
pub fn preview(rows: &[&ViewRow], triangles: &[Triangle], label: &str, output: &Path) -> anyhow::Result<()> {
    const SIZE: (usize, usize) = (560, 430);
    const HEADER: usize = 34;
    const PAPER: [u8; 3] = [246, 247, 249];
    if crate::storage::exists(output) {
        bail!("{} exists", output.display());
    }
    let files: Vec<PathBuf> = rows.iter().flat_map(|r| [PathBuf::from(&r.mask), PathBuf::from(&r.image)]).collect();
    let held = crate::inputs::hold_for_pool(&files)?;
    let panels: Vec<anyhow::Result<(Rgb, Rgb)>> = parallel_map(rows.len(), |n| {
        let row = rows[n];
        let mask = read_gray(Path::new(&row.mask))?;
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
        for p in (0..mask.data.len()).filter(|&p| mask.data[p] != 0) {
            (x0, y0, x1, y1) = (x0.min(p % mask.width), y0.min(p / mask.width), x1.max(p % mask.width), y1.max(p / mask.width));
        }
        if x0 == usize::MAX || x1 == x0 || y1 == y0 {
            bail!("empty mask for {}", row.name);
        }
        let centre = [(x0 + x1) as f64 / 2.0 + 0.5, (y0 + y1) as f64 / 2.0 + 0.5];
        let scale = 0.9 * (SIZE.0 as f64 / (x1 - x0) as f64).min(SIZE.1 as f64 / (y1 - y0) as f64);
        let photo = Rgb::open(Path::new(&row.image))?.scaled_view(centre, scale, SIZE.0, SIZE.1, PAPER);
        Ok((photo, shade(triangles, &camera(row), centre, scale, SIZE.0, SIZE.1)))
    });
    crate::inputs::release(held);
    let mut canvas = Rgb::filled(rows.len() * SIZE.0, 2 * (SIZE.1 + HEADER), PAPER);
    for (column, panel) in panels.into_iter().enumerate() {
        let (photo, shaded) = panel?;
        canvas.paste(&photo, column * SIZE.0, HEADER);
        canvas.paste(&shaded, column * SIZE.0, SIZE.1 + 2 * HEADER);
    }
    let ink = [26, 36, 49];
    canvas.text("Input photographs", 12, 26, 2, ink);
    canvas.text(&format!("{label}  ({} triangles)", with_thousands(triangles.len())), 12, SIZE.1 + HEADER + 26, 2, ink);
    canvas.save(output)
}

/// `np.unique(np.linspace(0, count - 1, wanted).round().astype(int))`.
fn spaced(count: usize, wanted: usize) -> Vec<usize> {
    let wanted = wanted.min(count);
    let mut out: Vec<usize> = (0..wanted)
        .map(|n| if wanted == 1 { 0.0 } else { n as f64 * ((count - 1) as f64 / (wanted - 1) as f64) })
        .map(|v| round_half_even(v) as usize)
        .collect();
    if wanted > 1 {
        *out.last_mut().unwrap() = count - 1;
    }
    out.dedup();
    out
}

/// Runs the check into a fresh directory and returns its report.
pub fn run(options: &Options, events: &EventLog) -> anyhow::Result<Value> {
    run_with(options, events, &crate::control::Control::none())
}

/// [`run`] that can be stopped: `control` is checked before every view is scored
/// and before each sheet. A stopped check returns `control::Stopped` and leaves no output directory.
pub fn run_with(options: &Options, events: &EventLog, control: &crate::control::Control) -> anyhow::Result<Value> {
    let result = checked(options, events, control);
    if result.as_ref().is_err_and(|error| error.downcast_ref::<crate::control::Stopped>().is_some()) {
        let _ = crate::storage::remove_dir_all(options.output);
    }
    result
}

fn checked(options: &Options, events: &EventLog, control: &crate::control::Control) -> anyhow::Result<Value> {
    let output = options.output;
    if crate::storage::exists(output) {
        bail!("output directory exists: {}", output.display());
    }
    crate::storage::create_dir_all(output)?;
    let rows = load_views(options.inputs)?;
    if rows.is_empty() {
        bail!("no views in {}", options.inputs.display());
    }
    let (triangles, _) = crate::stl::read_binary(options.mesh)?;
    let mut checked = spaced(rows.len(), options.check_views);
    let mut sets: Vec<Option<&Path>> = vec![None];
    if let Some(directory) = options.repaired_masks.filter(|d| crate::storage::is_dir(d)) {
        sets.push(Some(directory));
    }
    // np.linspace(0, len, views, endpoint=False, dtype=int)
    let views = options.preview_views.max(1);
    let named: Vec<usize> = (0..views).map(|n| (n as f64 * (rows.len() as f64 / views as f64)) as usize).collect();
    for n in 0..rows.len() {
        if named.contains(&n) && !checked.contains(&n) {
            checked.push(n);
        }
    }
    let overlay = options.preview_views > 0;
    let mut files: Vec<PathBuf> = Vec::new();
    for &n in &checked {
        let row = &rows[n];
        for set in &sets {
            files.push(set.map(|d| d.join(format!("{}.png", row.name))).unwrap_or_else(|| PathBuf::from(&row.mask)));
        }
        if overlay && named.contains(&n) {
            files.push(PathBuf::from(&row.image));
        }
    }
    let held = crate::inputs::hold_for_pool(&files)?;
    let scored: Vec<anyhow::Result<Scored>> = parallel_map(checked.len(), |n| {
        control.check()?;
        score_view(&triangles, &rows[checked[n]], &sets, overlay && named.contains(&checked[n]))
    });
    crate::inputs::release(held);
    let scored: Vec<Scored> = scored.into_iter().collect::<anyhow::Result<_>>()?;
    control.check()?;
    if overlay {
        let tiles: Vec<&Rgb> = scored.iter().filter_map(|s| s.tile.as_ref()).collect();
        let height = tiles.iter().map(|t| t.height).max().unwrap_or(1);
        let mut sheet = Rgb::filled(600 * tiles.len().max(1), height, [30; 3]);
        for (n, tile) in tiles.iter().enumerate() {
            sheet.paste(tile, 600 * n, 0);
        }
        sheet.save(&output.join("photo-overlay.png"))?;
    }
    events.progress(0.6, "Silhouettes compared")?;
    let names: Vec<&str> = checked.iter().map(|&n| rows[n].name.as_str()).collect();
    let of_set = |set: usize| -> Vec<f64> { scored.iter().map(|s| s.scores[set]).collect() };
    let mut report = json!({
        "mesh": options.mesh.display().to_string(), "triangles": triangles.len(), "views": rows.len(), "views_checked": checked.len(),
        "silhouette_iou_input_masks": summary(&names, &of_set(0)),
        "note": "photo agreement only; not scanner accuracy or physical scale",
    });
    if sets.len() > 1 {
        report["silhouette_iou_repaired_masks"] = summary(&names, &of_set(1));
    }
    events.metric("silhouette_iou_median", report["silhouette_iou_input_masks"]["median"].as_f64().unwrap_or(0.0))?;
    if overlay {
        events.artifact(
            "photo_overlay",
            &output.join("photo-overlay.png"),
            "Mesh outline against masks (green: both, red: mask only, blue: mesh only)",
            json!({}),
        )?;
        control.check()?;
        let picked: Vec<&ViewRow> = named.iter().map(|&n| &rows[n]).collect();
        preview(&picked, &triangles, "Reconstruction", &output.join("preview.png"))?;
        report["preview"] = json!(output.join("preview.png").display().to_string());
        events.artifact("preview_render", &output.join("preview.png"), "Photos and shaded reconstruction", json!({}))?;
    }
    crate::storage::write(output.join("result.json"), serde_json::to_string_pretty(&report)? + "\n")?;
    events.artifact("report", &output.join("result.json"), "Photo check", json!({}))?;
    Ok(report)
}

/// `crispmesh-dense check --inputs DIR --mesh STL --output DIR [--repaired-masks DIR] [--preview-views N] [--check-views N] [--events FILE]`.
pub fn main(arguments: &[String]) -> anyhow::Result<()> {
    const USAGE: &str = "usage: crispmesh-dense check --inputs DIR --mesh STL --output DIR [--repaired-masks DIR] [--preview-views N] [--check-views N] [--events FILE]";
    let mut values = std::collections::BTreeMap::new();
    let mut rest = arguments.iter();
    while let Some(flag) = rest.next() {
        if !["--inputs", "--mesh", "--output", "--repaired-masks", "--preview-views", "--check-views", "--events"].contains(&flag.as_str())
        {
            bail!("unknown argument {flag}\n{USAGE}");
        }
        values.insert(flag.clone(), rest.next().ok_or_else(|| anyhow!("{flag} needs a value"))?.clone());
    }
    let path = |name: &str| values.get(name).map(PathBuf::from);
    let need = |name: &str| path(name).ok_or_else(|| anyhow!("{name} is required\n{USAGE}"));
    let count = |name: &str, default: usize| -> anyhow::Result<usize> {
        values.get(name).map(|v| v.parse::<usize>().map_err(|_| anyhow!("{name} needs a number"))).unwrap_or(Ok(default))
    };
    let (inputs, mesh, output, repaired) = (need("--inputs")?, need("--mesh")?, need("--output")?, path("--repaired-masks"));
    let options = Options {
        inputs: &inputs,
        mesh: &mesh,
        output: &output,
        repaired_masks: repaired.as_deref(),
        preview_views: count("--preview-views", 3)?,
        check_views: count("--check-views", 24)?,
    };
    let events = EventLog::new(path("--events").as_deref(), "check");
    println!("{}", serde_json::to_string_pretty(&run(&options, &events)?)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spacing_follows_numpy() {
        assert_eq!(spaced(73, 24).len(), 24);
        assert_eq!((spaced(73, 24)[0], spaced(73, 24)[23]), (0, 72));
        // np.linspace(0, 72, 24).round(): 0, 3, 6, 9, 13, ...
        assert_eq!(&spaced(73, 24)[..5], &[0, 3, 6, 9, 13]);
        assert_eq!(spaced(5, 24), vec![0, 1, 2, 3, 4]);
        assert_eq!(spaced(1, 24), vec![0]);
    }
}
