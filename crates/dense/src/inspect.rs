//! Inspection sheets: every surface a run produces, drawn the same way, so that
//! the sheets can be flipped through to see what each step adds or loses.
//!
//! Three views (spread over the ring), each as the photo next to the surface
//! rendered through that view's camera, and below them one detail crop per
//! view: the square of the photo with the most fine detail inside the mask
//! (largest mean squared Laplacian), the same crop for every step. One sheet
//! per step (`inspect/NN-step.png`) and an overview with one row per step
//! (`inspect/steps.png`).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{bail, Context};
use serde_json::json;

use crate::events::EventLog;
use crate::inputs::ViewRow;
use crate::render::{shade, with_thousands, Rgb, ViewCamera};
use crate::scene::read_gray;

/// Size of one panel.
const PANEL: (usize, usize) = (240, 300);
const HEADER: usize = 26;
const PAPER: [u8; 3] = [246, 247, 249];
const INK: [u8; 3] = [26, 36, 49];

struct View {
    name: String,
    camera: ViewCamera,
    /// Whole object: centre and scale of the panel.
    whole: ([f64; 2], f64),
    /// Detail crop: centre and scale.
    detail: ([f64; 2], f64),
    photo: Rgb,
    photo_detail: Rgb,
}

struct Row {
    step: String,
    label: String,
    panels: Vec<(Rgb, Rgb)>,
}

/// The views and crops of a run, and the panels drawn so far.
pub struct Inspector {
    directory: PathBuf,
    views: Vec<View>,
    rows: Mutex<Vec<Row>>,
}

/// Mean squared Laplacian of the grey image over the square of side `side` at
/// every `stride`, where at least nine tenths of the square is mask: the
/// top-left corner of the most detailed square.
fn most_detailed(gray: &crate::inputs::Plane<u8>, mask: &crate::inputs::Plane<u8>, side: usize) -> Option<(usize, usize)> {
    let (w, h) = (gray.width, gray.height);
    if side < 4 || side >= w || side >= h {
        return None;
    }
    // Integral images of the squared Laplacian and of the mask.
    let mut detail = vec![0f64; (w + 1) * (h + 1)];
    let mut inside = vec![0f64; (w + 1) * (h + 1)];
    for y in 0..h {
        let mut row_detail = 0.0;
        let mut row_inside = 0.0;
        for x in 0..w {
            let at = y * w + x;
            let value = if x > 0 && y > 0 && x + 1 < w && y + 1 < h && mask.data[at] != 0 {
                let g = |xx: usize, yy: usize| f64::from(gray.data[yy * w + xx]);
                let laplacian = g(x - 1, y) + g(x + 1, y) + g(x, y - 1) + g(x, y + 1) - 4.0 * g(x, y);
                laplacian * laplacian
            } else {
                0.0
            };
            row_detail += value;
            row_inside += f64::from(u8::from(mask.data[at] != 0));
            detail[(y + 1) * (w + 1) + x + 1] = detail[y * (w + 1) + x + 1] + row_detail;
            inside[(y + 1) * (w + 1) + x + 1] = inside[y * (w + 1) + x + 1] + row_inside;
        }
    }
    let sum = |table: &[f64], x: usize, y: usize| {
        table[(y + side) * (w + 1) + x + side] - table[y * (w + 1) + x + side] - table[(y + side) * (w + 1) + x] + table[y * (w + 1) + x]
    };
    let stride = (side / 4).max(1);
    let area = (side * side) as f64;
    let mut best: Option<(f64, usize, usize)> = None;
    for y in (0..h - side).step_by(stride) {
        for x in (0..w - side).step_by(stride) {
            if sum(&inside, x, y) < 0.9 * area {
                continue;
            }
            let score = sum(&detail, x, y);
            if best.is_none_or(|b| score > b.0) {
                best = Some((score, x, y));
            }
        }
    }
    best.map(|(_, x, y)| (x, y))
}

impl Inspector {
    /// Picks three views of `rows` and their detail crops; sheets go to `directory`.
    pub fn new(rows: &[ViewRow], directory: &Path) -> anyhow::Result<Self> {
        if rows.is_empty() {
            bail!("no views to inspect");
        }
        let picks: Vec<usize> = (0..3usize).map(|n| n * rows.len() / 3).collect();
        let mut views = Vec::new();
        for &pick in &picks {
            let row = &rows[pick];
            let mask = read_gray(Path::new(&row.mask)).with_context(|| row.mask.clone())?;
            let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
            for p in (0..mask.data.len()).filter(|&p| mask.data[p] != 0) {
                (x0, y0, x1, y1) = (x0.min(p % mask.width), y0.min(p / mask.width), x1.max(p % mask.width), y1.max(p / mask.width));
            }
            if x0 == usize::MAX || x1 == x0 || y1 == y0 {
                bail!("empty mask for {}", row.name);
            }
            let centre = [(x0 + x1) as f64 / 2.0 + 0.5, (y0 + y1) as f64 / 2.0 + 0.5];
            let scale = 0.92 * (PANEL.0 as f64 / (x1 - x0) as f64).min(PANEL.1 as f64 / (y1 - y0) as f64);
            let gray = read_gray(Path::new(&row.image)).with_context(|| row.image.clone())?;
            let side = ((x1 - x0).max(y1 - y0) as f64 * 0.22).round() as usize;
            let (cx, cy) = most_detailed(&gray, &mask, side).unwrap_or((x0, y0));
            let detail_centre = [cx as f64 + side as f64 / 2.0, cy as f64 + side as f64 / 2.0];
            let detail_scale = (PANEL.0 as f64 / side as f64).min(PANEL.1 as f64 / side as f64);
            let image = Rgb::open(Path::new(&row.image))?;
            views.push(View {
                name: row.name.clone(),
                camera: ViewCamera { rotation: row.rotation, translation: row.translation, k: row.k },
                whole: (centre, scale),
                detail: (detail_centre, detail_scale),
                photo: image.scaled_view(centre, scale, PANEL.0, PANEL.1, PAPER),
                photo_detail: image.scaled_view(detail_centre, detail_scale, PANEL.0, PANEL.1, PAPER),
            });
        }
        crate::storage::create_dir_all(directory)?;
        // Where the views and crops are, for tools that draw other things (depth maps) at the same places.
        let record: Vec<serde_json::Value> = views
            .iter()
            .map(|v| {
                let side = PANEL.0 as f64 / v.detail.1;
                json!({"view": v.name, "centre": v.whole.0, "scale": v.whole.1, "detail_centre": v.detail.0, "detail_side": side})
            })
            .collect();
        crate::storage::write(directory.join("crops.json"), serde_json::to_string_pretty(&json!({"views": record}))? + "\n")?;
        Ok(Inspector { directory: directory.to_path_buf(), views, rows: Mutex::new(Vec::new()) })
    }

    /// Draws the sheet of one step from a mesh file and announces it; returns its path.
    pub fn add(&self, step: &str, label: &str, mesh: &Path, events: &EventLog) -> anyhow::Result<PathBuf> {
        let (triangles, _) = crate::stl::read_binary(mesh)?;
        let panels: Vec<(Rgb, Rgb)> = self
            .views
            .iter()
            .map(|view| {
                (
                    shade(&triangles, &view.camera, view.whole.0, view.whole.1, PANEL.0, PANEL.1),
                    shade(&triangles, &view.camera, view.detail.0, view.detail.1, PANEL.0, PANEL.1),
                )
            })
            .collect();
        // Sheet: per view a 2 x 2 block, photo | surface over photo detail | surface detail.
        let (w, h) = (PANEL.0, PANEL.1);
        let mut sheet = Rgb::filled(self.views.len() * 2 * w, HEADER + 2 * h, PAPER);
        for (n, (view, (surface, detail))) in self.views.iter().zip(&panels).enumerate() {
            let x = n * 2 * w;
            sheet.paste(&view.photo, x, HEADER);
            sheet.paste(surface, x + w, HEADER);
            sheet.paste(&view.photo_detail, x, HEADER + h);
            sheet.paste(detail, x + w, HEADER + h);
        }
        sheet.text(&format!("{label}  ({} triangles)", with_thousands(triangles.len())), 10, 20, 2, INK);
        let path = self.directory.join(format!("{step}.png"));
        sheet.save(&path)?;
        events.artifact("inspection_sheet", &path, label, json!({"step": step, "triangles": triangles.len()}))?;
        let mut rows = self.rows.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        rows.retain(|row| row.step != step);
        rows.push(Row { step: step.to_string(), label: label.to_string(), panels });
        rows.sort_by(|a, b| a.step.cmp(&b.step));
        Ok(path)
    }

    /// The overview: the photos, then one row per step drawn so far; announced as step `steps`.
    pub fn overview(&self, events: &EventLog) -> anyhow::Result<Option<PathBuf>> {
        let rows = self.rows.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if rows.is_empty() {
            return Ok(None);
        }
        let (w, h) = (PANEL.0, PANEL.1);
        let width = self.views.len() * 2 * w;
        let mut sheet = Rgb::filled(width, (rows.len() + 1) * (h + HEADER), PAPER);
        for (n, view) in self.views.iter().enumerate() {
            sheet.paste(&view.photo, n * 2 * w, HEADER);
            sheet.paste(&view.photo_detail, n * 2 * w + w, HEADER);
        }
        sheet.text("Photos and detail crops", 10, 20, 2, INK);
        for (r, row) in rows.iter().enumerate() {
            let y = (r + 1) * (h + HEADER);
            for (n, (surface, detail)) in row.panels.iter().enumerate() {
                sheet.paste(surface, n * 2 * w, y + HEADER);
                sheet.paste(detail, n * 2 * w + w, y + HEADER);
            }
            sheet.text(&row.label, 10, y + 20, 2, INK);
        }
        let path = self.directory.join("steps.png");
        sheet.save(&path)?;
        events.artifact(
            "inspection_sheet",
            &path,
            "All steps",
            json!({"step": "steps", "steps": rows.iter().map(|r| r.step.clone()).collect::<Vec<_>>()}),
        )?;
        Ok(Some(path))
    }
}

/// `crispmesh-dense inspect --inputs DIR --output DIR --mesh STEP=LABEL=FILE ...`: sheets of any meshes with
/// the views and crops of the inputs directory, and their overview, as a run draws them.
pub fn main(arguments: &[String]) -> anyhow::Result<()> {
    const USAGE: &str = "usage: crispmesh-dense inspect --inputs DIR --output DIR --mesh STEP=LABEL=FILE [--mesh ...]";
    let (mut inputs, mut output, mut meshes) = (None, None, Vec::new());
    let mut rest = arguments.iter();
    while let Some(flag) = rest.next() {
        let value = rest.next().ok_or_else(|| anyhow::anyhow!("{flag} needs a value\n{USAGE}"))?;
        match flag.as_str() {
            "--inputs" => inputs = Some(PathBuf::from(value)),
            "--output" => output = Some(PathBuf::from(value)),
            "--mesh" => {
                let parts: Vec<&str> = value.splitn(3, '=').collect();
                if parts.len() != 3 {
                    bail!("--mesh needs STEP=LABEL=FILE\n{USAGE}");
                }
                meshes.push((parts[0].to_string(), parts[1].to_string(), PathBuf::from(parts[2])));
            }
            _ => bail!("unknown argument {flag}\n{USAGE}"),
        }
    }
    let (Some(inputs), Some(output)) = (inputs, output) else { bail!("--inputs and --output are required\n{USAGE}") };
    let inspector = Inspector::new(&crate::inputs::load_views(&inputs)?, &output)?;
    let events = EventLog::none();
    for (step, label, mesh) in &meshes {
        println!("{}", inspector.add(step, label, mesh, &events)?.display());
    }
    if let Some(path) = inspector.overview(&events)? {
        println!("{}", path.display());
    }
    Ok(())
}

/// Whether sheets may be drawn: always natively; in a browser while the module's memory is below 2.5 GiB.
pub fn memory_allows() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        let bytes = core::arch::wasm32::memory_size(0) as u64 * 65536;
        bytes < (5u64 << 29)
    }
    #[cfg(not(target_arch = "wasm32"))]
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inputs::Plane;

    #[test]
    fn the_most_detailed_square_is_found_inside_the_mask() {
        let (w, h) = (64usize, 48usize);
        let mut gray = Plane::<u8>::new(w, h);
        let mut mask = Plane::<u8>::new(w, h);
        for y in 4..44 {
            for x in 4..60 {
                mask.data[y * w + x] = 1;
                gray.data[y * w + x] = 100;
            }
        }
        // A checkerboard (fine detail) around (40, 20), inside the mask.
        for y in 16..28 {
            for x in 36..48 {
                gray.data[y * w + x] = if (x + y) % 2 == 0 { 30 } else { 200 };
            }
        }
        let (x, y) = most_detailed(&gray, &mask, 12).unwrap();
        assert!((32..=40).contains(&x) && (12..=20).contains(&y), "{x} {y}");
        assert_eq!(most_detailed(&gray, &mask, 2), None);
    }
}
