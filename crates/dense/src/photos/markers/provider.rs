//! The `markers` camera provider inside the `photos` command: detection in
//! every staged photo, a pose per photo, the report, and the solution in the
//! mat's millimetre frame.

use anyhow::{anyhow, bail};
use serde_json::json;

use crate::photos::cameras::CameraProvider;
use crate::photos::options::Options;
use crate::photos::providers::{Provider, CAMERAS_MARKERS};
use crate::photos::run::Run;
use crate::photos::solution::{Lens, Solution};
use crate::photos::staging::capture_name;
use crate::photos::util;

use super::linalg::Camera;
use super::mat::Mat;
use super::{detect, pose};

pub struct Markers;

impl CameraProvider for Markers {
    fn info(&self) -> &'static Provider {
        &CAMERAS_MARKERS
    }

    fn check(&self, options: &Options) -> anyhow::Result<()> {
        let path = options.markers.mat.as_ref().ok_or_else(|| {
            anyhow!("--cameras markers needs the description of the printed mat: --markers-mat FILE.json (crispmesh-dense mat --size a4 --output DIR writes one)")
        })?;
        Mat::load(path).map(|_| ())
    }

    fn recover(&self, run: &mut Run, lens: Option<&Lens>) -> anyhow::Result<Solution> {
        let lens = *lens.ok_or_else(|| anyhow!("the markers provider needs the declared lens (--calibration)"))?;
        let (options, out) = (run.options, run.out);
        let mat = Mat::load(options.markers.mat.as_ref().ok_or_else(|| anyhow!("--markers-mat is missing"))?)?;
        let camera = Camera::from_lens(&lens);
        let (count, threads, small) = (options.photo_count, options.threads, options.timeouts.small);
        let names: Vec<String> = (0..count).map(capture_name).collect();
        let detections =
            run.internal("cameras", "markers-detect", options.timeouts.features, 0.04, 0.6, "Finding the mat's markers", count, |watch| {
                let work = |n: usize| -> anyhow::Result<Vec<detect::Detection>> {
                    let gray = super::gray_of(&out.join("work/photos").join(&names[n]))?;
                    Ok(detect::detect(&gray, &mat, Some(&camera), &detect::Settings::default()))
                };
                util::parallel(count, threads, work, watch)
            })?;
        let mut settings = pose::Settings { minimum_markers: options.markers.minimum_per_photo, ..pose::Settings::default() };
        let (results, aspect) = run.internal("cameras", "markers-pose", small, 0.6, 0.78, "Camera poses from the mat", 1, |_| {
            let mut results = pose::solve_views(&camera, &mat, &detections, &settings);
            let estimate = pose::estimate_aspect(&camera, &mat, &detections, &results);
            match options.markers.aspect {
                Some(given) => settings.aspect = given,
                None if options.markers.aspect_auto => settings.aspect = estimate.0,
                None => {}
            }
            if settings.aspect != 1.0 {
                results = pose::solve_views(&camera, &mat, &detections, &settings);
            }
            Ok((results, estimate))
        })?;
        let folder = out.join("sfm/markers");
        crate::photos::fs::create_dir_all(&folder)?;
        let found: Vec<_> =
            names.iter().zip(&detections).map(|(name, list)| json!({"photo": name, "markers": super::detections_json(list)})).collect();
        util::write_json(&folder.join("detections.json"), &json!(found), 1)?;
        util::write_json(&folder.join("mat.json"), &mat.to_json(), 1)?;
        let mut report = pose::report(&names, &results);
        report["print_aspect"] =
            json!({"used": settings.aspect, "estimate": aspect.0, "cost_at_1": aspect.1, "cost_at_estimate": aspect.2});
        report["mat"] = json!({"name": mat.name, "markers": mat.markers.len(), "marker_size_mm": mat.marker_size});
        util::write_json(&folder.join("report.json"), &report, 1)?;
        let mut summary = report.clone();
        if let Some(object) = summary.as_object_mut() {
            object.remove("views");
        }
        run.note("markers", summary);
        let missing: Vec<String> = names
            .iter()
            .zip(&results)
            .filter(|(_, r)| r.pose.is_none())
            .map(|(name, r)| format!("{name} ({})", r.reason.clone().unwrap_or_default()))
            .collect();
        if !missing.is_empty() {
            run.warn(format!("no pose from the mat for {} photos: {}", missing.len(), missing.join("; ")));
        }
        if (aspect.0 - 1.0).abs() > 0.003 && settings.aspect == 1.0 {
            run.warn(format!(
                "the photos fit a print whose height is {:.2} % off against its width; measure the mat or pass --markers-aspect auto",
                100.0 * (aspect.0 - 1.0)
            ));
        }
        let mut solution = pose::solution(&lens, &mat, &names, &results, settings.aspect);
        if solution.views.is_empty() {
            bail!("the mat's markers were not found in any photo (is --markers-mat the mat that was printed?)");
        }
        // Where the object is: the space above the mat that the masks allow.
        let registered: Vec<usize> = (0..count).filter(|&n| results[n].pose.is_some()).collect();
        let poses: Vec<pose::Pose> = registered.iter().filter_map(|&n| results[n].pose).collect();
        let points = run.internal("cameras", "markers-object", small, 0.78, 0.82, "Locating the object above the mat", 1, |_| {
            let masks: Vec<_> = registered
                .iter()
                .map(|&n| crate::scene::read_gray(&out.join("masks").join(&names[n])))
                .collect::<anyhow::Result<Vec<_>>>()?;
            let inside = |view: usize, pixel: [f64; 2]| -> Option<bool> {
                let mask = &masks[view];
                let (x, y) = (pixel[0].round(), pixel[1].round());
                (x >= 0.0 && y >= 0.0 && x < mask.width as f64 && y < mask.height as f64)
                    .then(|| mask.data[y as usize * mask.width + x as usize] > 127)
            };
            Ok(pose::object_points(&camera, &mat, &poses, &inside))
        })?;
        solution.object_points = Some(points);
        Ok(solution)
    }

    fn intermediates(&self) -> &'static [&'static str] {
        &[]
    }

    fn reads_staged_photos(&self) -> bool {
        true
    }
}

/// For `--masks threshold` with `--cameras markers`: hides the mat's visible markers in a photo's grey
/// values before the dark region is taken. Each photo's pose comes from its own markers, so this
/// needs nothing from the cameras stage. `None` for every other combination of providers.
pub fn mask_preparation(options: &Options) -> anyhow::Result<Option<Box<crate::photos::staging::Preparation>>> {
    use crate::photos::options::{CameraChoice, MaskChoice};
    if options.cameras != CameraChoice::Markers || options.masks != MaskChoice::Threshold {
        return Ok(None);
    }
    let Some(mat) = &options.markers.mat else { return Ok(None) };
    let file = options.calibration.as_deref().map(crate::photos::calibration::load_calibration).transpose()?;
    let field_of_view = options.lens_field_of_view;
    if file.is_none() && field_of_view.is_none() {
        return Ok(None);
    }
    let mat = Mat::load(mat)?;
    Ok(Some(Box::new(move |gray: &mut crate::inputs::Plane<u8>| -> anyhow::Result<Option<u32>> {
        let (width, height) = (gray.width as u32, gray.height as u32);
        let calibration = match (&file, field_of_view) {
            (Some(calibration), _) => calibration.clone(),
            (None, degrees) => crate::photos::calibration::field_of_view_calibration(degrees.unwrap_or_default(), width, height)?,
        };
        let scaled = crate::photos::calibration::scale_calibration(&calibration, width, height)?;
        let camera = Camera::from_lens(&scaled.lens(width, height));
        let found = detect::detect(gray, &mat, Some(&camera), &detect::Settings::default());
        let Some(pose) = pose::solve_view(&camera, &mat, &found, &pose::Settings::default()).pose else { return Ok(None) };
        // The level comes from the object zone of the mat, where there is only paper and object;
        // Otsu's level of the whole photo would follow whatever the mat lies on.
        let level = pose::object_level(gray, &mat, &camera, &pose);
        pose::hide_markers(gray, &mat, &camera, &pose);
        let Some((level, surface)) = level else { return Ok(None) };
        if surface.is_some_and(|grey| grey < level as f64) {
            bail!(
                "the surface around the mat is as dark as the object (grey {:.0}, the object is below {level}): a threshold cannot tell them apart. \
                 Put the mat on a light surface, or use --masks external-sam or --masks import:DIR",
                surface.unwrap_or(0.0)
            );
        }
        Ok(Some(level))
    })))
}
