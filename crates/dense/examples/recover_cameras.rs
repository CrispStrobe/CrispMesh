//! Offline experimental camera recovery from own photo correspondences.
//! Not an app provider. No angle/order or supplied-pose prior is consumed.
use anyhow::{bail, Context, Result};
use crispmesh_dense::photos::{
    incremental::{self, BundleSolver, Initializer, RecoveryOptions, Resection, SeedSelection, TrackSelection},
    solution::{colmap_parameters, Lens, Solution},
    turntable::solver::Matches,
};
use serde_json::Value;
use std::{path::Path, path::PathBuf};

fn quaternion(r: [[f64; 3]; 3]) -> [f64; 4] {
    let trace = r[0][0] + r[1][1] + r[2][2];
    if trace > 0. {
        let s = (trace + 1.).sqrt() * 2.;
        return [s / 4., (r[2][1] - r[1][2]) / s, (r[0][2] - r[2][0]) / s, (r[1][0] - r[0][1]) / s];
    }
    let i = if r[0][0] > r[1][1] && r[0][0] > r[2][2] {
        0
    } else if r[1][1] > r[2][2] {
        1
    } else {
        2
    };
    let j = (i + 1) % 3;
    let k = (i + 2) % 3;
    let s = (1. + r[i][i] - r[j][j] - r[k][k]).sqrt() * 2.;
    let mut q = [0.; 4];
    q[0] = (r[k][j] - r[j][k]) / s;
    q[i + 1] = s / 4.;
    q[j + 1] = (r[i][j] + r[j][i]) / s;
    q[k + 1] = (r[i][k] + r[k][i]) / s;
    q
}

fn export(solution: &Solution, output: &Path, report: &Value) -> Result<()> {
    if output.exists() {
        bail!("output must be a fresh directory");
    }
    // COLMAP names occupy a text record. Refuse unsafe/multi-token names.
    if solution
        .views
        .iter()
        .any(|v| v.source.chars().any(char::is_whitespace) || Path::new(&v.source).file_name().and_then(|n| n.to_str()) != Some(&v.source))
    {
        bail!("COLMAP export requires basename-only photo names without whitespace");
    }
    std::fs::create_dir_all(output)?;
    let lens = solution.lens;
    let parameters = colmap_parameters(&lens).map(|x| x.to_string()).join(" ");
    std::fs::write(output.join("cameras.txt"), format!("1 FULL_OPENCV {} {} {parameters}\n", lens.width, lens.height))?;
    let mut observations = vec![Vec::new(); solution.views.len()];
    let mut points = String::new();
    for (index, point) in solution.landmarks.iter().enumerate() {
        let id = index + 1;
        let mut track = String::new();
        for &(view, xy) in &point.observations {
            track += &format!(" {} {}", view + 1, observations[view].len());
            observations[view].push((xy, id));
        }
        points += &format!("{id} {} {} {} 128 128 128 0{track}\n", point.position[0], point.position[1], point.position[2]);
    }
    std::fs::write(output.join("points3D.txt"), points)?;
    let mut images = String::new();
    for (index, view) in solution.views.iter().enumerate() {
        let q = quaternion(view.rotation);
        images += &format!(
            "{} {} {} {} {} {} {} {} 1 {}\n",
            index + 1,
            q[0],
            q[1],
            q[2],
            q[3],
            view.translation[0],
            view.translation[1],
            view.translation[2],
            view.source
        );
        for (xy, point) in &observations[index] {
            // Solution uses integer pixel centres. COLMAP uses half centres.
            images += &format!("{} {} {point} ", xy[0] + 0.5, xy[1] + 0.5);
        }
        images.push('\n');
    }
    std::fs::write(output.join("images.txt"), images)?;
    std::fs::write(output.join("report.json"), serde_json::to_vec_pretty(report)?)?;
    Ok(())
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut options = RecoveryOptions::default();
    let (mut photos, mut masks, mut inputs): (Option<PathBuf>, Option<PathBuf>, Option<PathBuf>) = (None, None, None);
    while let Some(key) = args.next() {
        if key == "--help" {
            println!("recover_cameras --matches CACHE --output NEW_MODEL [--initializer eight|five] [--resection linear|p3p|consensus] [--tracks robust|split] [--seeds score|diverse] [--seed-budget 32] [--measurement-error-scale 1] [--bundle-solver auto|dense|iterative|off]\nOptional native inputs: --photos DIR --masks DIR --inputs NEW_DIR (production undistortion/crop writer).\nOffline experimental own-photo recovery; no quality guarantee or app integration.");
            return Ok(());
        }
        let value = args.next().context("each option requires a value")?;
        match key.as_str() {
            "--matches" => input = Some(value.into()),
            "--output" => output = Some(value.into()),
            "--photos" => photos = Some(value.into()),
            "--masks" => masks = Some(value.into()),
            "--inputs" => inputs = Some(value.into()),
            "--bundle-solver" => {
                options.bundle_solver = match value.as_str() {
                    "auto" => BundleSolver::Auto,
                    "dense" => BundleSolver::Dense,
                    "iterative" => BundleSolver::Iterative,
                    "off" => BundleSolver::Off,
                    _ => bail!("unknown bundle solver"),
                }
            }
            "--initializer" => {
                options.initializer = match value.as_str() {
                    "eight" => Initializer::EightPoint,
                    "five" => Initializer::FivePoint,
                    _ => bail!("unknown initializer"),
                }
            }
            "--resection" => {
                options.resection = match value.as_str() {
                    "linear" => Resection::Linear,
                    "p3p" => Resection::P3p,
                    "consensus" => Resection::P3pConsensus,
                    _ => bail!("unknown resection"),
                }
            }
            "--tracks" => {
                options.track_selection = match value.as_str() {
                    "robust" => TrackSelection::Robust,
                    "split" => TrackSelection::Split,
                    _ => bail!("unknown tracks"),
                }
            }
            "--seeds" => {
                options.seed_selection = match value.as_str() {
                    "score" => SeedSelection::Score,
                    "diverse" => SeedSelection::Diverse,
                    _ => bail!("unknown seed selection"),
                }
            }
            "--seed-budget" => options.maximum_seed_attempts = value.parse()?,
            "--measurement-error-scale" => options.measurement_error_scale = value.parse()?,
            _ => bail!("unknown option {key}"),
        }
    }
    let input = input.context("--matches is required")?;
    let output = output.context("--output is required")?;
    if output.exists() {
        bail!("output must be a fresh directory");
    }
    if [photos.is_some(), masks.is_some(), inputs.is_some()].iter().any(|&x| x)
        && !(photos.is_some() && masks.is_some() && inputs.is_some())
    {
        bail!("--photos, --masks and --inputs must be supplied together");
    }
    if inputs.as_ref().is_some_and(|path| path.exists() || output.starts_with(path)) {
        bail!("native inputs must be a fresh directory, not a parent of the model output");
    }
    let json: Value = serde_json::from_slice(&std::fs::read(input)?)?;
    let matches = Matches::from_json(&json)?;
    let names: Vec<String> = serde_json::from_value(json["names"].clone())?;
    let width: u32 = serde_json::from_value(json["lens"]["width"].clone())?;
    let height: u32 = serde_json::from_value(json["lens"]["height"].clone())?;
    let lens = Lens {
        width,
        height,
        pixels: serde_json::from_value(json["lens"]["pixels"].clone())?,
        k: serde_json::from_value(json["lens"]["k"].clone())?,
    };
    let mut previous = usize::MAX;
    let recovered = incremental::recover_with_options(&matches, &lens, &names, options, &mut |n, total| {
        if n != previous {
            eprintln!("registered {n}/{total}");
            previous = n;
        }
        Ok(())
    })?;
    export(&recovered.solution, &output, &recovered.report)?;
    if let (Some(photos), Some(masks), Some(inputs)) = (photos, masks, inputs) {
        let aliases = output.join("source-mask-aliases");
        std::fs::create_dir(&aliases)?;
        for view in &recovered.solution.views {
            let source = [masks.join(&view.source), masks.join(format!("{}.png", view.source))]
                .into_iter()
                .find(|path| path.is_file())
                .context("own source mask missing")?;
            std::fs::copy(source, aliases.join(&view.source))?;
        }
        let result = crispmesh_dense::photos::scene_writer::write_scene_cropped(
            &recovered.solution,
            &photos,
            None,
            &aliases,
            &inputs,
            2,
            Some(0.15),
            &mut |_| Ok(()),
        );
        std::fs::remove_dir_all(&aliases)?;
        let report = result?;
        std::fs::write(output.join("native-input-report.json"), serde_json::to_vec_pretty(&report)?)?;
    }
    println!("{}", serde_json::to_string(&recovered.report)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crispmesh_dense::photos::solution::{read_colmap, Landmark, View};
    #[test]
    fn exported_cameras_and_observations_round_trip_without_half_pixel_shift() {
        let root = std::env::temp_dir().join(format!("crispmesh-recovery-export-{}", std::process::id()));
        let output = root.join("model");
        let rotation = [[-1., 0., 0.], [0., 1., 0.], [0., 0., -1.]];
        let lens = Lens { width: 64, height: 48, pixels: [80., 80., 31.5, 23.5], k: [0.; 3] };
        let solution = Solution {
            lens,
            views: vec![View { id: "1".into(), source: "photo.jpg".into(), rotation, centre: [0., 0., 2.], translation: [0., 0., 2.] }],
            unregistered: vec![],
            landmarks: vec![Landmark { position: [0., 0., 0.], observations: vec![(0, [31.5, 23.5])] }],
            lens_locked: Some(true),
            scale: None,
            object_points: None,
        };
        export(&solution, &output, &serde_json::json!({})).unwrap();
        let restored = read_colmap(&output, &["photo.jpg".into()]).unwrap();
        assert_eq!(restored.lens, lens);
        assert_eq!(restored.landmarks[0].observations, solution.landmarks[0].observations);
        assert_eq!(restored.views[0].rotation, rotation);
        assert_eq!(restored.views[0].translation, solution.views[0].translation);
        assert!(export(&solution, &output, &serde_json::json!({})).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
