//! The marker mat: a printed sheet of square fiducials around a free zone for
//! the object. Its description (`crispmesh_marker_mat_v1`) carries everything a
//! detector needs: every marker's id, bit pattern and corner coordinates in
//! millimetres.
//!
//! Frame: origin at the page centre, +X to the right and +Y to the top of the
//! printed page, +Z out of the page towards the viewer; right-handed.
//!
//! The markers are ArUco markers of OpenCV's `DICT_4X4_50` (4 x 4 bits, one
//! cell of black border, minimum Hamming distance 4 over ids and rotations).
//! The 50 codes below were read from OpenCV 4.10
//! (`cv2.aruco.getPredefinedDictionary(cv2.aruco.DICT_4X4_50)`, bits row by row
//! from the top-left, 1 = white), so any ArUco detector reads the mat too.

use std::path::Path;

use anyhow::{anyhow, bail, Context};
use serde_json::{json, Value};

pub const SCHEMA: &str = "crispmesh_marker_mat_v1";

pub const DICT_4X4_50: [u16; 50] = [
    0xb532, 0x0f9a, 0x332d, 0x9946, 0x549e, 0x79cd, 0x9e2e, 0xc4f2, 0xfeda, 0xcf56, 0xf991, 0x11a7, 0x0eb7, 0x2a0f, 0x24b1, 0x263e, 0x4665,
    0x6600, 0x6c5e, 0x76af, 0x868b, 0xb02b, 0xccd5, 0xdd82, 0xfe47, 0x9471, 0xace4, 0xa554, 0x2123, 0x346f, 0x4415, 0x57b2, 0x9ecf, 0xf0cb,
    0x08ae, 0x0929, 0x1875, 0x04ff, 0x0df6, 0x1c5a, 0x1718, 0x2a28, 0x328c, 0x38b2, 0x24e8, 0x2eeb, 0x2d3f, 0x4b64, 0x502e, 0x5013,
];

/// One printed marker. `code` holds `bits * bits` cells row by row from the
/// top-left (most significant first), 1 = white. Corners are top-left,
/// top-right, bottom-right, bottom-left as printed, in millimetres.
#[derive(Debug, Clone, PartialEq)]
pub struct Marker {
    pub id: u32,
    pub code: u64,
    pub corners: [[f64; 2]; 4],
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mat {
    pub name: String,
    pub page: [f64; 2],
    /// Cells per side of the code (4 for `DICT_4X4_50`); a border of one black cell surrounds it.
    pub bits: usize,
    pub dictionary: String,
    pub marker_size: f64,
    /// Radius of the marker-free zone around the origin, where the object stands.
    pub object_radius: f64,
    pub markers: Vec<Marker>,
}

/// `(name, width, height, marker size, pitch, object zone radius)` in millimetres.
pub const SIZES: [(&str, f64, f64, f64, f64, f64); 3] =
    [("a4", 210.0, 297.0, 24.0, 34.0, 50.0), ("letter", 215.9, 279.4, 24.0, 34.0, 50.0), ("a3", 297.0, 420.0, 32.0, 45.0, 75.0)];

/// Unprinted edge every printer leaves.
const MARGIN: f64 = 10.0;

impl Mat {
    /// The mat for a named page size.
    pub fn standard(name: &str) -> anyhow::Result<Mat> {
        let lower = name.to_lowercase();
        let Some(&(name, width, height, marker, pitch, radius)) = SIZES.iter().find(|s| s.0 == lower) else {
            bail!("unknown mat size {name:?}; offered: {}", SIZES.map(|s| s.0).join(", "));
        };
        Mat::grid(name, [width, height], marker, pitch, radius)
    }

    /// A centred grid of markers `pitch` apart, leaving out those that reach into the object zone.
    pub fn grid(name: &str, page: [f64; 2], marker: f64, pitch: f64, object_radius: f64) -> anyhow::Result<Mat> {
        if !(marker > 0.0 && pitch >= marker * 1.3 && object_radius >= 0.0) {
            bail!("a mat needs a positive marker size and a pitch of at least 1.3 marker sizes (white space between markers)");
        }
        let count = |extent: f64| (((extent - 2.0 * MARGIN - marker) / pitch).floor() as i64 + 1).max(0);
        let (columns, rows) = (count(page[0]), count(page[1]));
        let mut markers = Vec::new();
        for row in 0..rows {
            for column in 0..columns {
                let x = (column as f64 - (columns - 1) as f64 / 2.0) * pitch;
                let y = ((rows - 1) as f64 / 2.0 - row as f64) * pitch;
                let h = marker / 2.0;
                // Nearest point of the marker square to the origin.
                let nearest = ((x.abs() - h).max(0.0).powi(2) + (y.abs() - h).max(0.0).powi(2)).sqrt();
                if nearest < object_radius + marker / 6.0 {
                    continue;
                }
                let id = markers.len();
                if id >= DICT_4X4_50.len() {
                    bail!("the mat needs more than {} markers; use larger markers or a larger pitch", DICT_4X4_50.len());
                }
                markers.push(Marker {
                    id: id as u32,
                    code: DICT_4X4_50[id] as u64,
                    corners: [[x - h, y + h], [x + h, y + h], [x + h, y - h], [x - h, y - h]],
                });
            }
        }
        if markers.len() < 8 {
            bail!("only {} markers fit on the page", markers.len());
        }
        Ok(Mat {
            name: name.to_string(),
            page,
            bits: 4,
            dictionary: "DICT_4X4_50".to_string(),
            marker_size: marker,
            object_radius,
            markers,
        })
    }

    /// Whether cell (`row`, `column`) of a marker's code is white.
    pub fn white(&self, marker: &Marker, row: usize, column: usize) -> bool {
        (marker.code >> (self.bits * self.bits - 1 - (row * self.bits + column))) & 1 == 1
    }

    /// Reflectance at a point of the page plane: 0 black, 1 white, `None` beyond the page.
    pub fn colour(&self, x: f64, y: f64) -> Option<f64> {
        if x.abs() > self.page[0] / 2.0 || y.abs() > self.page[1] / 2.0 {
            return None;
        }
        let cells = (self.bits + 2) as f64;
        for marker in &self.markers {
            let [left, top] = marker.corners[0];
            let (u, v) = ((x - left) / self.marker_size * cells, (top - y) / self.marker_size * cells);
            if u >= 0.0 && v >= 0.0 && u < cells && v < cells {
                let (column, row) = (u as usize, v as usize);
                let inner = (1..=self.bits).contains(&column) && (1..=self.bits).contains(&row);
                return Some(if inner && self.white(marker, row - 1, column - 1) { 1.0 } else { 0.0 });
            }
        }
        Some(1.0)
    }

    /// End points of the scale bar: 100 mm where the object zone is wide enough, else the largest
    /// multiple of 10 mm that fits. It lies inside the object zone, away from every marker, and is
    /// covered by the object during a capture.
    pub fn scale_bar(&self) -> ([f64; 2], [f64; 2]) {
        let length = (((2.0 * self.object_radius - 16.0) / 10.0).floor() * 10.0).clamp(10.0, 100.0);
        let y = -self.object_radius / 3.0;
        ([-length / 2.0, y], [length / 2.0, y])
    }

    pub fn to_json(&self) -> Value {
        let (from, to) = self.scale_bar();
        let digits = self.bits * self.bits / 4;
        json!({
            "schema": SCHEMA, "name": self.name, "unit": "mm",
            "frame": "origin at the page centre, +X to the right and +Y to the top of the printed page, +Z out of the page; right-handed",
            "page": {"width": self.page[0], "height": self.page[1]},
            "dictionary": {"name": self.dictionary, "bits": self.bits, "border_cells": 1,
                           "code": "cells row by row from the top-left, most significant first, 1 = white, as hexadecimal"},
            "marker_size": self.marker_size, "object_radius": self.object_radius,
            "scale_bar": {"from": from, "to": to, "length": to[0] - from[0]},
            "corner_order": "top-left, top-right, bottom-right, bottom-left as printed",
            "markers": self.markers.iter().map(|m| json!({
                "id": m.id, "code": format!("{:0digits$x}", m.code),
                "corners": m.corners.map(|c| [c[0], c[1], 0.0]),
            })).collect::<Vec<_>>(),
        })
    }

    pub fn from_json(value: &Value) -> anyhow::Result<Mat> {
        if value["schema"] != SCHEMA || value["unit"] != "mm" {
            bail!("not a marker mat description (schema {SCHEMA}, unit mm)");
        }
        let number = |v: &Value, what: &str| v.as_f64().filter(|x| x.is_finite()).ok_or_else(|| anyhow!("marker mat: {what} is missing"));
        let bits = value["dictionary"]["bits"]
            .as_u64()
            .filter(|b| (3..=8).contains(b))
            .ok_or_else(|| anyhow!("marker mat: dictionary.bits must be 3..8"))? as usize;
        if value["dictionary"]["border_cells"].as_u64().unwrap_or(1) != 1 {
            bail!("marker mat: only a border of one cell is supported");
        }
        let marker_size = number(&value["marker_size"], "marker_size")?;
        let mut markers = Vec::new();
        for entry in value["markers"].as_array().ok_or_else(|| anyhow!("marker mat: markers are missing"))? {
            let id = entry["id"].as_u64().ok_or_else(|| anyhow!("marker mat: a marker has no id"))? as u32;
            let code = u64::from_str_radix(entry["code"].as_str().unwrap_or("-"), 16)
                .map_err(|_| anyhow!("marker mat: marker {id} has no code"))?;
            let list = entry["corners"]
                .as_array()
                .filter(|c| c.len() == 4)
                .ok_or_else(|| anyhow!("marker mat: marker {id} needs four corners"))?;
            let mut corners = [[0.0; 2]; 4];
            for (corner, item) in corners.iter_mut().zip(list) {
                *corner = [number(&item[0], "a corner")?, number(&item[1], "a corner")?];
                if item.get(2).and_then(Value::as_f64).unwrap_or(0.0) != 0.0 {
                    bail!("marker mat: marker {id} is not in the plane z = 0");
                }
            }
            markers.push(Marker { id, code, corners });
        }
        let mut ids: Vec<u32> = markers.iter().map(|m| m.id).collect();
        ids.sort_unstable();
        ids.dedup();
        if ids.len() != markers.len() || markers.is_empty() {
            bail!("marker mat: marker ids must be unique and there must be at least one");
        }
        Ok(Mat {
            name: value["name"].as_str().unwrap_or("mat").to_string(),
            page: [number(&value["page"]["width"], "page.width")?, number(&value["page"]["height"], "page.height")?],
            bits,
            dictionary: value["dictionary"]["name"].as_str().unwrap_or_default().to_string(),
            marker_size,
            object_radius: value["object_radius"].as_f64().unwrap_or(0.0),
            markers,
        })
    }

    pub fn load(path: &Path) -> anyhow::Result<Mat> {
        let text = crate::photos::fs::read_to_string(path).with_context(|| path.display().to_string())?;
        Mat::from_json(&serde_json::from_str(&text).with_context(|| path.display().to_string())?)
            .with_context(|| path.display().to_string())
    }

    /// Black rectangles of the drawing as `(x, y of the top-left, width, height)` in page millimetres, plus thin grey guide rectangles.
    fn rectangles(&self) -> (Vec<[f64; 4]>, Vec<[f64; 4]>) {
        let cells = self.bits + 2;
        let cell = self.marker_size / cells as f64;
        let mut black = Vec::new();
        for marker in &self.markers {
            let [left, top] = marker.corners[0];
            for row in 0..cells {
                // Runs of black cells per row keep the drawing small and free of hairlines between cells.
                let mut column = 0;
                while column < cells {
                    let is_black =
                        |c: usize| !((1..=self.bits).contains(&c) && (1..=self.bits).contains(&row) && self.white(marker, row - 1, c - 1));
                    if !is_black(column) {
                        column += 1;
                        continue;
                    }
                    let start = column;
                    while column < cells && is_black(column) {
                        column += 1;
                    }
                    black.push([left + start as f64 * cell, top - row as f64 * cell, (column - start) as f64 * cell, cell]);
                }
            }
        }
        let (from, to) = self.scale_bar();
        black.push([from[0], from[1] + 0.15, to[0] - from[0], 0.3]);
        let ticks = ((to[0] - from[0]) / 10.0).round() as usize;
        for tick in 0..=ticks {
            let tall = if tick % 5 == 0 { 3.0 } else { 1.5 };
            black.push([from[0] + tick as f64 * 10.0 - 0.15, from[1], 0.3, tall]);
        }
        // Centre cross and the two axes, in light grey so that no detector takes them for part of a marker.
        let grey = vec![[-6.0, 0.1, 12.0, 0.2], [-0.1, 6.0, 0.2, 12.0]];
        (black, grey)
    }

    /// Two short lines below the scale bar.
    fn captions(&self) -> [String; 2] {
        let (from, to) = self.scale_bar();
        [format!("crispmesh marker mat {}", self.name), format!("print at 100 %: this bar is {:.0} mm", to[0] - from[0])]
    }

    pub fn svg(&self) -> String {
        let (w, h) = (self.page[0], self.page[1]);
        let (black, grey) = self.rectangles();
        let mut out = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}mm\" height=\"{h}mm\" viewBox=\"0 0 {w} {h}\">\n<rect width=\"{w}\" height=\"{h}\" fill=\"#ffffff\"/>\n"
        );
        let mut draw = |list: &[[f64; 4]], colour: &str| {
            for r in list {
                out += &format!(
                    "<rect x=\"{:.4}\" y=\"{:.4}\" width=\"{:.4}\" height=\"{:.4}\" fill=\"{colour}\"/>\n",
                    r[0] + w / 2.0,
                    h / 2.0 - r[1],
                    r[2],
                    r[3]
                );
            }
        };
        draw(&grey, "#b0b0b0");
        draw(&black, "#000000");
        out += &format!(
            "<circle cx=\"{:.3}\" cy=\"{:.3}\" r=\"{:.3}\" fill=\"none\" stroke=\"#b0b0b0\" stroke-width=\"0.2\" stroke-dasharray=\"1 1\"/>\n",
            w / 2.0,
            h / 2.0,
            self.object_radius
        );
        let (from, _) = self.scale_bar();
        for (line, caption) in self.captions().iter().enumerate() {
            out += &format!(
                "<text x=\"{:.3}\" y=\"{:.3}\" font-family=\"Helvetica, Arial, sans-serif\" font-size=\"2.6\" text-anchor=\"middle\" fill=\"#606060\">{caption}</text>\n",
                w / 2.0,
                h / 2.0 - from[1] + 7.5 + 3.6 * line as f64
            );
        }
        out += "</svg>\n";
        out
    }

    /// A one-page PDF with the page at its true size.
    pub fn pdf(&self) -> Vec<u8> {
        const POINTS: f64 = 72.0 / 25.4;
        let (w, h) = (self.page[0], self.page[1]);
        let (black, grey) = self.rectangles();
        let mut content = String::new();
        let draw = |list: &[[f64; 4]], level: f64, content: &mut String| {
            *content += &format!("{level} g\n");
            for r in list {
                let (x, y) = ((r[0] + w / 2.0) * POINTS, (r[1] - r[3] + h / 2.0) * POINTS);
                *content += &format!("{x:.4} {y:.4} {:.4} {:.4} re f\n", r[2] * POINTS, r[3] * POINTS);
            }
        };
        draw(&grey, 0.69, &mut content);
        draw(&black, 0.0, &mut content);
        let (from, _) = self.scale_bar();
        // Helvetica averages about half its size per character; start so that the lines are roughly centred.
        let size = 2.6 * POINTS;
        content += "0.38 g\n";
        for (line, caption) in self.captions().iter().enumerate() {
            let start = (w / 2.0) * POINTS - caption.len() as f64 * size * 0.25;
            let y = (from[1] - 7.5 - 3.6 * line as f64 + h / 2.0) * POINTS;
            content += &format!("BT /F1 {size:.3} Tf {start:.3} {y:.3} Td ({caption}) Tj ET\n");
        }
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {:.4} {:.4}] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>",
                w * POINTS,
                h * POINTS
            ),
            format!("<< /Length {} >>\nstream\n{content}endstream", content.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (n, object) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n{object}\nendobj\n", n + 1).bytes());
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
        for offset in offsets {
            out.extend(format!("{offset:010} 00000 n \n").bytes());
        }
        out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).bytes());
        out
    }

    /// The page as an 8-bit grey raster at `dpi` dots per inch: `(width, height, pixels)`.
    pub fn raster(&self, dpi: f64) -> (usize, usize, Vec<u8>) {
        let scale = dpi / 25.4;
        let (width, height) = ((self.page[0] * scale).round() as usize, (self.page[1] * scale).round() as usize);
        let mut pixels = vec![255u8; width * height];
        let (black, grey) = self.rectangles();
        for (list, level) in [(&grey, 176u8), (&black, 0u8)] {
            for r in list.iter() {
                let x0 = ((r[0] + self.page[0] / 2.0) * scale).round().max(0.0) as usize;
                let y0 = ((self.page[1] / 2.0 - r[1]) * scale).round().max(0.0) as usize;
                let x1 = (((r[0] + r[2] + self.page[0] / 2.0) * scale).round() as usize).clamp(x0 + 1, width.max(x0 + 1)).min(width);
                let y1 = (((self.page[1] / 2.0 - r[1] + r[3]) * scale).round() as usize).clamp(y0 + 1, height.max(y0 + 1)).min(height);
                for y in y0.min(height)..y1 {
                    pixels[y * width + x0.min(width)..y * width + x1].fill(level);
                }
            }
        }
        (width, height, pixels)
    }

    /// Writes `mat.json`, `mat.svg`, `mat.pdf` and `mat.png` (300 dpi) into `folder`.
    pub fn write(&self, folder: &Path) -> anyhow::Result<()> {
        crate::photos::fs::create_dir_all(folder).with_context(|| folder.display().to_string())?;
        let stem = format!("crispmesh-marker-mat-{}", self.name);
        let text = serde_json::to_string_pretty(&self.to_json())? + "\n";
        crate::photos::fs::write(folder.join(format!("{stem}.json")), text)?;
        crate::photos::fs::write(folder.join(format!("{stem}.svg")), self.svg())?;
        crate::photos::fs::write(folder.join(format!("{stem}.pdf")), self.pdf())?;
        let (width, height, pixels) = self.raster(300.0);
        crate::photos::util::save_gray(&folder.join(format!("{stem}.png")), width, height, pixels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_mats_keep_the_object_zone_free_and_round_trip() {
        for (name, expected) in [("a4", 28), ("letter", 30), ("a3", 38)] {
            let mat = Mat::standard(name).unwrap();
            assert_eq!(mat.markers.len(), expected, "{name}");
            for marker in &mat.markers {
                for corner in marker.corners {
                    assert!(corner[0].abs() <= mat.page[0] / 2.0 - MARGIN + 1e-9 && corner[1].abs() <= mat.page[1] / 2.0 - MARGIN + 1e-9);
                    assert!((corner[0].powi(2) + corner[1].powi(2)).sqrt() > mat.object_radius);
                }
                // Top-left, top-right, bottom-right, bottom-left with +Y up.
                assert!(marker.corners[0][0] < marker.corners[1][0] && marker.corners[0][1] > marker.corners[3][1]);
            }
            assert_eq!(Mat::from_json(&mat.to_json()).unwrap(), mat);
            let (from, to) = mat.scale_bar();
            assert_eq!(to[0] - from[0], if name == "a3" { 100.0 } else { 80.0 });
            assert!(from[0].hypot(from[1] - 3.0) < mat.object_radius - 5.0);
        }
        assert!(Mat::standard("a0").is_err());
        assert!(Mat::grid("tiny", [60.0, 60.0], 24.0, 34.0, 10.0).is_err());
    }

    #[test]
    fn cells_follow_the_dictionary() {
        let mat = Mat::standard("a4").unwrap();
        let marker = &mat.markers[0];
        // Id 0 of DICT_4X4_50: 1011 / 0101 / 0011 / 0010.
        let rows: Vec<String> = (0..4).map(|r| (0..4).map(|c| if mat.white(marker, r, c) { '1' } else { '0' }).collect()).collect();
        assert_eq!(rows, ["1011", "0101", "0011", "0010"]);
        let [left, top] = marker.corners[0];
        let cell = mat.marker_size / 6.0;
        assert_eq!(mat.colour(left + 0.5 * cell, top - 0.5 * cell), Some(0.0)); // border
        assert_eq!(mat.colour(left + 1.5 * cell, top - 1.5 * cell), Some(1.0)); // first cell, white
        assert_eq!(mat.colour(left + 2.5 * cell, top - 1.5 * cell), Some(0.0));
        assert_eq!(mat.colour(0.0, 0.0), Some(1.0));
        assert_eq!(mat.colour(500.0, 0.0), None);
        // The raster shows the same cells; the vector files name the true page size.
        let (width, height, pixels) = mat.raster(254.0);
        assert_eq!((width, height), (2100, 2970));
        let at = |x: f64, y: f64| pixels[((mat.page[1] / 2.0 - y) * 10.0) as usize * width + ((x + mat.page[0] / 2.0) * 10.0) as usize];
        assert_eq!((at(left + 0.5 * cell, top - 0.5 * cell), at(left + 1.5 * cell, top - 1.5 * cell)), (0, 255));
        assert!(mat.svg().contains("width=\"210mm\" height=\"297mm\""));
        let pdf = String::from_utf8_lossy(&mat.pdf()).to_string();
        assert!(pdf.starts_with("%PDF-1.4") && pdf.contains("/MediaBox [0 0 595.2756 841.8898]") && pdf.trim_end().ends_with("%%EOF"));
        let xref: usize = pdf[pdf.rfind("startxref").unwrap() + 10..].lines().next().unwrap().parse().unwrap();
        assert!(pdf[xref..].starts_with("xref"));
    }
}
