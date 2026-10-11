//! Preview sheets of the stage: ports of `colour_sheet` and `preview`.
//!
//! These are pictures for people, not inputs of later stages. Tiles are resized
//! with the crate's float resampler and then rounded, where Pillow resamples
//! 8-bit data in fixed point, so single grey levels may differ from the
//! reference sheets.

use std::path::Path;

use crate::inputs::{Inputs, Plane};

use super::level::{resize, Filter, LevelView};

/// Resizes an interleaved 8-bit image with a bicubic filter (Pillow's default for `resize`).
fn resize_u8(data: &[u8], width: usize, height: usize, channels: usize, new_width: usize, new_height: usize) -> Vec<u8> {
    let mut out = vec![0u8; new_width * new_height * channels];
    for c in 0..channels {
        let plane = Plane { width, height, data: data.iter().skip(c).step_by(channels).map(|&v| v as f32).collect() };
        let resized = resize(&plane, new_width, new_height, Filter::Bicubic);
        for (n, &v) in resized.data.iter().enumerate() {
            out[n * channels + c] = (v + 0.5).clamp(0.0, 255.0) as u8;
        }
    }
    out
}

fn tile_height(width: usize, height: usize, target: usize) -> usize {
    (crate::inputs::round_half_even(target as f64 * height as f64 / width as f64) as usize).max(1)
}

/// Photo crops with coloured overlays, side by side. `layers(view)` yields
/// (boolean image, rgb) pairs painted in order at 70 % opacity.
pub fn colour_sheet(
    path: &Path,
    inputs: &Inputs,
    picks: &[usize],
    layers: impl Fn(usize) -> Vec<(Plane<u8>, [f32; 3])>,
) -> anyhow::Result<()> {
    let mut tiles: Vec<(Vec<u8>, usize)> = Vec::new();
    for &i in picks {
        let (w, h) = (inputs.masks[i].width, inputs.masks[i].height);
        let layers = layers(i);
        let b = inputs.boxes[i];
        let (x0, y0) = (b[0].max(0) as usize, b[1].max(0) as usize);
        let (x1, y1) = ((b[2].min(w as i64)).max(0) as usize, (b[3].min(h as i64)).max(0) as usize);
        let (cw, ch) = (x1.saturating_sub(x0).max(1), y1.saturating_sub(y0).max(1));
        let mut crop = Vec::with_capacity(cw * ch * 3);
        for y in y0..y0 + ch {
            for x in x0..x0 + cw {
                let (x, y) = (x.min(w - 1), y.min(h - 1));
                let g = inputs.gray_at(i, x, y).unwrap_or(0.0);
                let mut pixel = [g.clamp(0.0, 1.4) / 1.4; 3];
                for (mask, colour) in &layers {
                    if mask.data[y * w + x] != 0 {
                        for c in 0..3 {
                            pixel[c] = 0.3 * pixel[c] + 0.7 * colour[c];
                        }
                    }
                }
                crop.extend(pixel.map(|v| (255.0 * v) as u8));
            }
        }
        let th = tile_height(cw, ch, 600);
        tiles.push((resize_u8(&crop, cw, ch, 3, 600, th), th));
    }
    let height = tiles.iter().map(|t| t.1).max().unwrap_or(1);
    let width = 600 * tiles.len().max(1);
    let mut sheet = vec![30u8; width * height * 3];
    for (n, (tile, th)) in tiles.iter().enumerate() {
        for y in 0..*th {
            let target = (y * width + 600 * n) * 3;
            sheet[target..target + 1800].copy_from_slice(&tile[y * 1800..(y + 1) * 1800]);
        }
    }
    crate::storage::save_png(path, width, height, 3, &sheet)
}

/// `np.percentile(values, q)` on unsorted float32 data, in float64.
fn percentile(values: &mut [f32], q: f64) -> f64 {
    values.sort_unstable_by(f32::total_cmp);
    crate::inputs::percentile_sorted_f32(values, q)
}

/// Depth sheet: per picked view the photo, the depth and a depth shading, stacked (`preview`).
pub fn depth_sheet(path: &Path, level: &[LevelView], depths: &[Plane<f32>], picks: &[usize]) -> anyhow::Result<()> {
    let mut tiles: Vec<(Vec<u8>, usize)> = Vec::new();
    for &i in picks {
        let (view, depth) = (&level[i], &depths[i]);
        let (w, h) = (view.width, view.height);
        let valid = |x: usize, y: usize| depth.data[y * w + x] > 0.0;
        let mut inside: Vec<f32> = depth.data.iter().cloned().filter(|&d| d > 0.0).collect();
        let (lo, hi) = if inside.is_empty() { (0.0, 1.0) } else { (percentile(&mut inside.clone(), 1.0), percentile(&mut inside, 99.0)) };
        // np.gradient of the depth with holes as NaN: central differences inside, one-sided at the borders.
        let value = |x: usize, y: usize| if valid(x, y) { depth.data[y * w + x] as f64 } else { f64::NAN };
        let gradient = |x: usize, y: usize, along_x: bool| -> f64 {
            let (position, extent) = if along_x { (x, w) } else { (y, h) };
            let at = |p: usize| if along_x { value(p, y) } else { value(x, p) };
            if extent < 2 {
                0.0
            } else if position == 0 {
                at(1) - at(0)
            } else if position == extent - 1 {
                at(extent - 1) - at(extent - 2)
            } else {
                (at(position + 1) - at(position - 1)) / 2.0
            }
        };
        let softness = ((hi - lo) / 80.0).powi(2);
        let mut tile = vec![0u8; 3 * w * h];
        for y in 0..h {
            for x in 0..w {
                let photo = if view.mask.data[y * w + x] != 0 { (view.gray.data[y * w + x] as f64).clamp(0.0, 1.0) } else { 0.12 };
                let shown = if valid(x, y) { ((depth.data[y * w + x] as f64 - lo) / (hi - lo + 1e-9)).clamp(0.0, 1.0) } else { 0.12 };
                let (gy, gx) = (gradient(x, y, false), gradient(x, y, true));
                let shade = 0.5 + 0.5 * (-gx - gy) / (gx * gx + gy * gy + softness).sqrt();
                let shade = if shade.is_finite() { shade } else { 0.12 };
                for (n, v) in [photo, shown, shade].into_iter().enumerate() {
                    tile[y * 3 * w + n * w + x] = (255.0 * v) as u8;
                }
            }
        }
        let th = tile_height(3 * w, h, 1200);
        tiles.push((resize_u8(&tile, 3 * w, h, 1, 1200, th), th));
    }
    let height: usize = tiles.iter().map(|t| t.1).sum::<usize>().max(1);
    let mut sheet = Vec::with_capacity(1200 * height);
    for (tile, _) in &tiles {
        sheet.extend_from_slice(tile);
    }
    sheet.resize(1200 * height, 0);
    crate::storage::save_png(path, 1200, height, 1, &sheet)
}
