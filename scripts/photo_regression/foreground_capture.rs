//! Desktop batch adapter for the app's own foreground preview implementation.
//! Full-frame detection preserves arbitrary capture order; tracking is a fallback.
//! Isolated-spike recovery follows core/foregroundPreview.ts.
//! No supplied camera, object box or reference mask is consumed.

#[cfg(target_os = "macos")]
#[allow(dead_code)]
#[path = "../../apps/studio/src-tauri/src/foreground.rs"]
mod foreground;

#[cfg(target_os = "macos")]
#[link(name = "Foundation", kind = "framework")]
extern "C" {}

#[derive(Clone)]
#[cfg(target_os = "macos")]
struct Detection {
    data: Vec<u8>,
    size: [f64; 2],
    bounds: [f64; 4],
}

#[cfg(target_os = "macos")]
impl Detection {
    fn decode(data: Vec<u8>) -> Result<Self, String> {
        if data.len() < 32 {
            return Err("invalid-foreground-response".into());
        }
        let v: Vec<f64> = data[..24]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|s| u32::from_le_bytes(*s) as f64)
            .collect();
        Ok(Self {
            data,
            size: [v[0], v[1]],
            bounds: [v[2], v[3], v[4], v[5]],
        })
    }
    fn width(&self) -> f64 {
        (self.bounds[2] - self.bounds[0]) / self.size[0]
    }
    fn height(&self) -> f64 {
        (self.bounds[3] - self.bounds[1]) / self.size[1]
    }
    fn area(&self) -> f64 {
        self.width() * self.height()
    }
    fn region(&self) -> [f64; 4] {
        let [l, t, r, b] = self.bounds;
        let [w, h] = self.size;
        let margin = (0.7 * (r - l).max(b - t)).ceil();
        [
            (l - margin).max(0.) / w,
            (t - margin).max(0.) / h,
            (r + margin).min(w) / w,
            (b + margin).min(h) / h,
        ]
    }
}

#[cfg(target_os = "macos")]
fn detect(
    path: &std::path::Path,
    region: Option<[f64; 4]>,
    fallback: bool,
    tracking_first: bool,
) -> Result<Detection, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let mut proposals = Vec::new();
    if fallback && !tracking_first {
        proposals.push((1., None));
    }
    if let Some(region) = region {
        proposals.push((1., Some(region)));
    }
    if fallback {
        if tracking_first { proposals.push((1., None)); }
        proposals.extend([(0.7, None), (0.4, None)]);
    }
    for (focus, region) in proposals {
        match foreground::preview_region(&bytes, focus, 0.5, region) {
            Ok(data) => return Detection::decode(data),
            Err(reason) if reason == "foreground-not-found" => {}
            Err(reason) => return Err(reason),
        }
    }
    Err("foreground-not-found".into())
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{fs, path::Path};
    let args: Vec<_> = std::env::args_os().collect();
    let tracking_first = args.get(3).is_some_and(|v| v == "--tracking-first");
    if !(args.len() == 3 || (args.len() == 4 && tracking_first)) {
        return Err("usage: foreground-capture PHOTOS OUTPUT [--tracking-first]".into());
    }
    if !foreground::available() {
        return Err("macOS 14 foreground detection is unavailable".into());
    }
    let output = Path::new(&args[2]);
    if output.exists() {
        return Err("output must be a new directory".into());
    }
    fs::create_dir_all(output)?;
    let mut files: Vec<_> = fs::read_dir(&args[1])?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<_, _>>()?;
    files.retain(|p| {
        p.extension().is_some_and(|e| {
            matches!(
                e.to_string_lossy().to_lowercase().as_str(),
                "jpg" | "jpeg" | "png"
            )
        })
    });
    files.sort();
    if !(8..=1536).contains(&files.len()) {
        return Err("expected 8..1536 original photos".into());
    }
    let mut previous = None;
    let mut detections = Vec::new();
    let mut failures = Vec::new();
    for path in &files {
        match detect(path, previous, true, tracking_first) {
            Ok(value) => {
                previous = Some(value.region());
                detections.push(Some(value));
                failures.push(None);
            }
            Err(reason) => {
                detections.push(None);
                failures.push(Some(reason));
            }
        }
        println!("foreground {} / {}", detections.len(), files.len());
    }
    // An isolated support/table selection must expand in BOTH dimensions, and
    // its neighbours must agree. Sustained distance changes remain untouched.
    let initial = detections.clone();
    let mut recovered = Vec::new();
    for i in 1..files.len() - 1 {
        let (Some(before), Some(current), Some(after)) =
            (&initial[i - 1], &initial[i], &initial[i + 1])
        else {
            continue;
        };
        let low = before.area().min(after.area());
        let high = before.area().max(after.area());
        if high / low > 2.
            || current.area() <= high * 4.
            || current.width() <= 2. * before.width().max(after.width())
            || current.height() <= 2. * before.height().max(after.height())
        {
            continue;
        }
        detections[i] = None;
        for anchor in [after, before] {
            if let Ok(candidate) = detect(&files[i], Some(anchor.region()), false, tracking_first) {
                if candidate.area() >= low / 2. && candidate.area() <= high * 2. {
                    detections[i] = Some(candidate);
                    recovered.push(i);
                    break;
                }
            }
        }
    }
    let mut rows = Vec::new();
    for (i, (path, detection)) in files.iter().zip(&detections).enumerate() {
        let name = path.file_name().unwrap().to_string_lossy();
        if let Some(detection) = detection {
            let target = output.join(format!("{name}.instance-1.png"));
            let pending = target.with_extension("pending");
            fs::write(&pending, &detection.data[24..])?;
            fs::rename(pending, target)?;
            rows.push(serde_json::json!({"photo":name,"box":detection.bounds,"accepted":true,"spike_recovered":recovered.contains(&i)}));
        } else {
            rows.push(serde_json::json!({"photo":name,"accepted":false,"reason":failures[i]}));
        }
    }
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"method":"App foreground preview with independent detection and isolated-spike recovery","tracking_first":tracking_first,"photos":rows,"reference_used":false,"manual_regions":false}),
        )?,
    )?;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("foreground-capture requires macOS 14 or newer");
    std::process::exit(1);
}
