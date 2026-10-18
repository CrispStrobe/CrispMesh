//! The whole photo stage on a small synthetic capture, without external programs:
//! threshold masks, cameras imported from an AliceVision scene.

use super::*;
use crate::photos::calibration::{alicevision_intrinsic_fields, load_calibration, parse_calibration, scale_calibration};
use crate::photos::options::resolve;

const COUNT: usize = 16;
const SIZE: (u32, u32) = (96, 72);

fn words(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn threshold_backdrop_masks_stop_before_cameras_and_keep_the_sheet() {
    let root = crate::photos::options::tests::scratch("backdrop-mask-gate");
    synthetic(&root);
    for n in 0..COUNT {
        let photo = image::RgbImage::from_fn(SIZE.0, SIZE.1, |x, y| {
            image::Rgb(if (44..52).contains(&x) && (32..40).contains(&y) { [205; 3] } else { [30; 3] })
        });
        photo.save(root.join(format!("photos/shot_{n}_rgb.png"))).unwrap();
    }
    let (finished, events) = start(&root, "rejected", &["--stop-after", "masks"]);
    assert_eq!(finished.code, 1);
    assert_eq!(finished.report["status"], "failed");
    assert!(finished.report["reasons"][0].as_str().unwrap().contains("90% of every photo"));
    assert!(root.join("rejected/mask-contact-sheet.png").is_file());
    assert!(root.join("rejected/masks-report.json").is_file());
    assert!(!root.join("rejected/inputs").exists());
    assert!(events.iter().any(|e| e["type"] == "artifact" && e["kind"] == "mask_sheet"));
    assert!(events.iter().any(|e| e["type"] == "error" && e["stage"] == "masks"));
    assert!(!events.iter().any(|e| e["stage"] == "cameras" && e["type"] == "stage_started"));
    // A single clear foreground view keeps the conservative every-photo gate open.
    let normal = image::RgbImage::from_fn(SIZE.0, SIZE.1, |x, y| {
        image::Rgb(if (30..60).contains(&x) && (25..50).contains(&y) { [30; 3] } else { [205; 3] })
    });
    normal.save(root.join("photos/shot_0_rgb.png")).unwrap();
    let (mixed, _) = start(&root, "mixed", &["--stop-after", "masks"]);
    assert_eq!(mixed.code, 0);
    assert_eq!(mixed.report["masks"]["views_filling_over_90_percent_of_photo"].as_array().unwrap().len(), COUNT - 1);
    std::fs::remove_dir_all(root).unwrap();
}

/// Sixteen small photos of a dark blob, a lens file and an AliceVision scene with exact cameras on a ring.
fn synthetic(root: &Path) {
    let _ = std::fs::remove_dir_all(root);
    std::fs::create_dir_all(root.join("photos")).unwrap();
    let lens = json!({"schema": "crispmesh_lens_calibration_v1", "model": "radialk3", "calibration_width": 48, "calibration_height": 36,
                      "fx": 60.0, "fy": 60.5, "cx": 23.25, "cy": 17.5, "k1": -0.05, "k2": 0.01, "k3": 0.002});
    util::write_json(&root.join("lens.json"), &lens, 1).unwrap();
    let scaled = scale_calibration(&parse_calibration(&lens).unwrap(), SIZE.0, SIZE.1).unwrap();
    let (p, k) = ([scaled.fx, scaled.fy, scaled.cx, scaled.cy], scaled.k);
    let mut intrinsic =
        json!({"intrinsicId": "5", "type": "pinhole", "distortionType": "radialk3", "width": "96", "height": "72", "sensorWidth": "36"});
    let Value::Object(fields) = alicevision_intrinsic_fields(&scaled.lens(SIZE.0, SIZE.1), 36.0) else { unreachable!() };
    intrinsic.as_object_mut().unwrap().extend(fields);
    let points: Vec<[f64; 3]> =
        (0..40).map(|n| [0.3 * ((n * 7) as f64).sin(), 0.3 * ((n * 3) as f64).cos(), 0.3 * ((n * 5) as f64).sin()]).collect();
    let (mut views, mut poses, mut cameras) = (Vec::new(), Vec::new(), Vec::new());
    for n in 0..COUNT {
        // The photos are named so that natural order differs from lexical order.
        let photo = image::RgbImage::from_fn(SIZE.0, SIZE.1, |x, y| {
            let inside = ((x as f64 - 48.0) / (22.0 + (n % 3) as f64)).powi(2) + ((y as f64 - 36.0) / 18.0).powi(2) < 1.0;
            image::Rgb(if inside { [30 + ((x * 7 + y * 13) % 25) as u8; 3] } else { [205; 3] })
        });
        photo.save(root.join(format!("photos/shot_{n}_rgb.png"))).unwrap();
        let angle = (n as f64 * 360.0 / COUNT as f64).to_radians();
        let centre = [3.0 * angle.sin(), 0.4, 3.0 * angle.cos()];
        let length = (centre[0] * centre[0] + centre[1] * centre[1] + centre[2] * centre[2]).sqrt();
        let forward = centre.map(|v| -v / length);
        let across = [forward[2], 0.0, -forward[0]];
        let norm = (across[0] * across[0] + across[2] * across[2]).sqrt();
        let right = across.map(|v| v / norm);
        let down = [
            forward[1] * right[2] - forward[2] * right[1],
            forward[2] * right[0] - forward[0] * right[2],
            forward[0] * right[1] - forward[1] * right[0],
        ];
        let rotation = [right, down, forward];
        let id = (100 + n).to_string();
        views.push(json!({"viewId": id, "poseId": id, "intrinsicId": "5", "path": format!("/elsewhere/shot_{n}_rgb.png")}));
        let flat: Vec<f64> = (0..3).flat_map(|column| (0..3).map(move |row| rotation[row][column])).collect();
        poses.push(json!({"poseId": id, "pose": {"transform": {"rotation": flat, "center": centre}}}));
        cameras.push((id, rotation, centre));
    }
    let structure: Vec<Value> = points
        .iter()
        .enumerate()
        .map(|(n, x)| {
            let observations: Vec<Value> = cameras
                .iter()
                .map(|(id, r, c)| {
                    let d = [x[0] - c[0], x[1] - c[1], x[2] - c[2]];
                    let q = r.map(|row| row[0] * d[0] + row[1] * d[1] + row[2] * d[2]);
                    let (u, v) = (q[0] / q[2], q[1] / q[2]);
                    let rr = u * u + v * v;
                    let gain = 1.0 + k[0] * rr + k[1] * rr * rr + k[2] * rr * rr * rr;
                    json!({"observationId": id, "x": [p[0] * u * gain + p[2], p[1] * v * gain + p[3]]})
                })
                .collect();
            json!({"landmarkId": n.to_string(), "X": x, "observations": observations})
        })
        .collect();
    let scene = json!({"version": ["1", "2", "14"], "views": views, "poses": poses, "intrinsics": [intrinsic], "structure": structure});
    util::write_json(&root.join("solved.sfm"), &scene, 1).unwrap();
}

fn start(root: &Path, output: &str, more: &[&str]) -> (Finished, Vec<Value>) {
    let mut arguments =
        words(&["--photos", &root.join("photos").to_string_lossy(), "--calibration", &root.join("lens.json").to_string_lossy()]);
    arguments.extend(words(&["--output", &root.join(output).to_string_lossy(), "--minimum-free-gib", "0", "--masks", "threshold"]));
    arguments.extend(words(&["--threshold-level", "70", "--cameras", &format!("import:{}", root.join("solved.sfm").display())]));
    arguments.extend(words(more));
    let options = resolve(&arguments, &|_| None).unwrap();
    let events_path = options.output.join("events.jsonl");
    let finished = run(&options, &EventLog::new(Some(&events_path), "cameras"), &events_path, None).unwrap();
    let text = std::fs::read_to_string(&events_path).unwrap();
    (finished, text.lines().map(|line| serde_json::from_str(line).unwrap()).collect())
}

#[test]
fn whole_stage_with_threshold_masks_and_imported_cameras() {
    let root = std::env::temp_dir().join(format!("crispmesh-photos-run-{}", std::process::id()));
    synthetic(&root);
    let (finished, events) = start(&root, "out", &[]);
    let out = root.join("out");
    assert_eq!(finished.code, 0, "{}", finished.report["reasons"]);
    let report = util::read_json(&out.join("frontend.json")).unwrap();
    assert_eq!(report["status"], "complete");
    assert_eq!((report["providers"]["masks"].as_str(), report["providers"]["cameras"].as_str()), (Some("threshold"), Some("import")));
    assert_eq!((report["gates"]["passed"].as_bool(), report["gates"]["registered_views"].as_u64()), (Some(true), Some(COUNT as u64)));
    assert!(report["gates"]["reprojection_pixels"]["max"].as_f64().unwrap() < 1e-6);
    assert!((report["gates"]["ring"]["largest_angular_gap_deg"].as_f64().unwrap() - 22.5).abs() < 1e-6);
    assert_eq!(report["supplied_poses_used"], true);
    let steps: Vec<&str> = report["steps"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert_eq!(steps, ["coarse", "cleanup", "publish-masks", "contrast", "import-cameras", "audit", "scene", "overlay"]);
    // The scene: views in capture order under their capture names, undistorted images and masks, points.
    let cameras = util::read_json(&out.join("inputs/cameras.json")).unwrap();
    let views = cameras["views"].as_array().unwrap();
    assert_eq!(views.len(), COUNT);
    assert_eq!((views[10]["source"].as_str(), views[10]["name"].as_str()), (Some("capture_0010.png"), Some("view_110")));
    for view in views {
        let (image, mask) = (view["image"].as_str().unwrap(), view["mask"].as_str().unwrap());
        assert!(out.join("inputs").join(image).is_file() && out.join("inputs").join(mask).is_file());
    }
    assert_eq!(crate::npz::read_npy(&out.join("inputs/sparse_points.npy")).unwrap().shape, [40, 3]);
    for file in [
        "masks/capture_0015.png",
        "mask-contact-sheet.png",
        "sparse-overlay.png",
        "sfm/gates.json",
        "sfm/camera-audit.json",
        "logs/01-coarse.log",
    ] {
        assert!(out.join(file).is_file(), "{file}");
    }
    assert!(!out.join("work/photos").exists() && !out.join("work/contrast").exists());
    assert_eq!(util::read_json(&out.join("photo-map.json")).unwrap()["photos"][10]["source"], "shot_10_rgb.png");
    // Events: two stages, their metrics and artifacts, no run-level events.
    let kinds: Vec<(&str, &str)> = events.iter().map(|e| (e["stage"].as_str().unwrap_or("-"), e["type"].as_str().unwrap())).collect();
    assert_eq!(kinds.first(), Some(&("masks", "stage_started")));
    assert_eq!(kinds.last(), Some(&("cameras", "stage_finished")));
    assert_eq!(kinds.iter().filter(|k| k.1 == "stage_finished").count(), 2);
    let artifacts: Vec<(&str, &str)> =
        events.iter().filter(|e| e["type"] == "artifact").map(|e| (e["kind"].as_str().unwrap(), e["path"].as_str().unwrap())).collect();
    assert_eq!(
        artifacts,
        [("mask_sheet", "mask-contact-sheet.png"), ("sparse_overlay", "sparse-overlay.png"), ("report", "frontend.json")]
    );
    let metrics: Vec<&str> = events.iter().filter(|e| e["type"] == "metric").map(|e| e["name"].as_str().unwrap()).collect();
    assert_eq!(metrics[..4], ["mask_area_median_pixels", "mask_area_median_fraction", "registered_views", "input_photos"]);
    assert!(events.iter().all(|e| e["type"] != "run_started" && e["type"] != "error"));
    // The dense stage's loader reads the scene.
    let inputs = crate::inputs::Inputs::load(&out.join("inputs"), &crate::config::DenseConfig::default()).unwrap();
    assert_eq!(inputs.count(), COUNT);

    // A gate that fails: exit code 2, the reasons in the report and in an error event, no scene.
    let (finished, events) = start(&root, "rejected", &["--maximum-angular-gap-deg", "10"]);
    assert_eq!(finished.code, 2);
    assert_eq!(finished.report["reasons"], json!(["largest gap between neighbouring cameras is 22.5 degrees; limit 10.0"]));
    assert!(!root.join("rejected/inputs").exists());
    assert!(events.iter().any(|e| e["type"] == "error" && e["message"].as_str().unwrap().starts_with("cameras rejected: largest gap")));

    // A cancel file next to the event log stops the run with exit code 1.
    let photos = root.join("photos").to_string_lossy().to_string();
    let front = root.join("cancelled/front").to_string_lossy().to_string();
    let options = resolve(
        &words(&["--photos", &photos, "--output", &front, "--masks", "threshold", "--stop-after", "masks", "--minimum-free-gib", "0"]),
        &|_| None,
    )
    .unwrap();
    std::fs::create_dir_all(root.join("cancelled")).unwrap();
    std::fs::write(root.join("cancelled/cancel"), "").unwrap();
    let events_path = root.join("cancelled/events.jsonl");
    let finished = run(&options, &EventLog::new(Some(&events_path), "masks"), &events_path, None).unwrap();
    assert_eq!((finished.code, finished.report["status"].as_str()), (1, Some("failed")));
    assert!(finished.report["reasons"][0].as_str().unwrap().contains("cancelled"));
    std::fs::remove_file(root.join("cancelled/cancel")).unwrap();
    // Masks only: no camera provider is needed or checked.
    let options = Options { output: root.join("cancelled/masks-only"), ..options };
    let finished = run(&options, &EventLog::new(Some(&events_path), "masks"), &events_path, None).unwrap();
    assert_eq!((finished.code, finished.report["stopped_after"].as_str()), (0, Some("masks")));
    assert!(root.join("cancelled/masks-only/masks/capture_0000.png").is_file());
    std::fs::remove_dir_all(&root).unwrap();
}

/// With `CRISPMESH_ALICEVISION_TESTS=1` and `CRISPMESH_ALICEVISION` (plus what the build needs, e.g.
/// `CRISPMESH_ALICEVISION_LIBRARY_PATH`): the real `cameraInit` lists the synthetic photos and its
/// scene takes the declared lens.
#[test]
fn alicevision_lists_views_when_installed() {
    if std::env::var("CRISPMESH_ALICEVISION_TESTS").as_deref() != Ok("1") {
        return;
    }
    let root = std::env::temp_dir().join(format!("crispmesh-photos-alicevision-{}", std::process::id()));
    synthetic(&root);
    let out = root.join("out");
    std::fs::create_dir_all(out.join("work/contrast")).unwrap();
    std::fs::create_dir_all(out.join("sfm")).unwrap();
    for n in 0..COUNT {
        std::fs::copy(root.join(format!("photos/shot_{n}_rgb.png")), out.join("work/contrast").join(capture_name(n))).unwrap();
    }
    let arguments =
        words(&["--photos", &root.join("photos").to_string_lossy(), "--calibration", &root.join("lens.json").to_string_lossy()]);
    let arguments: Vec<String> =
        arguments.into_iter().chain(words(&["--output", &out.to_string_lossy(), "--masks", "threshold"])).collect();
    let options = resolve(&arguments, &|name| std::env::var(name).ok()).unwrap();
    crate::photos::cameras::camera_provider(&options).check(&options).unwrap();
    let commands = crate::photos::cameras::alicevision_commands(&options, &|name| std::env::var(name).ok()).unwrap();
    let outcome =
        bounded(&commands[0].command, &root.join("cameraInit.log"), Duration::from_secs(120), &commands[0].environment, &out, &mut || None)
            .unwrap();
    assert!(outcome.succeeded(), "{}", log_tail(&root.join("cameraInit.log"), 2000));
    let scene = util::read_json(&out.join("sfm/cameraInit-uncalibrated.sfm")).unwrap();
    assert_eq!(scene["views"].as_array().unwrap().len(), COUNT);
    let lens = scale_calibration(&load_calibration(&root.join("lens.json")).unwrap(), SIZE.0, SIZE.1).unwrap().lens(SIZE.0, SIZE.1);
    let calibrated = crate::photos::calibration::calibrated_scene(&scene, &lens).unwrap();
    assert_eq!(calibrated["intrinsics"][0]["locked"], "true");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_turn_is_judged_by_its_own_photos() {
    let view = |name: &str, count: usize, p95: f64| json!({"name": name, "reprojection_pixels": {"count": count, "p95": p95}});
    let audit = json!({
        "passed": false,
        "reasons": ["insufficient distinct landmark support per registered view", "landmark behind camera"],
        "policy": {"minimum_observations_per_view": 20, "maximum_per_view_reprojection_p95_pixels": 4.0},
        "per_view": [view("capture_0000.png", 200, 1.0), view("capture_0001.png", 12, 1.0),
                     view("capture_0002.png", 200, 1.0), view("capture_0003.png", 200, 6.0)],
    });
    let names: Vec<String> = (0..4).map(capture_name).collect();
    let first = turn_audit(&audit, &names[0..2]);
    assert_eq!(first["reasons"], json!(["landmark behind camera", "insufficient distinct landmark support per registered view"]));
    let second = turn_audit(&audit, &names[2..4]);
    assert_eq!(second["reasons"], json!(["landmark behind camera", "per-view reprojection exceeds declared bound"]));
    let clean = turn_audit(
        &json!({"passed": false, "reasons": ["insufficient distinct landmark support per registered view"],
                                   "policy": audit["policy"], "per_view": audit["per_view"]}),
        &names[2..3],
    );
    assert_eq!((clean["passed"].as_bool(), clean["reasons"].as_array().map(Vec::len)), (Some(true), Some(0)));
}
