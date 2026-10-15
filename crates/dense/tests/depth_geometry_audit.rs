//! Independent analytic geometry through production projection, GPU fusion and meshing.
//! No dataset geometry, cameras, masks or depths are used.
use crispmesh_dense::{
    config::DenseConfig,
    events::EventLog,
    fusion,
    gpu::{block_on, Gpu},
    hull::Hull,
    inputs::{Camera, Plane},
    mesh::{self, field::Volume},
    stereo::level::LevelView,
    stl,
};

const BOXES: [([f64; 3], [f64; 3]); 3] = [
    ([0.0; 3], [0.8; 3]),
    ([-0.3, -0.85, 0.0], [0.12, 0.25, 0.12]), // removed recess
    ([0.3, -0.88, 0.0], [0.12, 0.10, 0.12]),  // raised feature
];
fn inside(p: [f64; 3], b: usize) -> bool {
    (0..3).all(|a| (p[a] - BOXES[b].0[a]).abs() < BOXES[b].1[a])
}
fn solid(p: [f64; 3]) -> bool {
    (inside(p, 0) && !inside(p, 1)) || inside(p, 2)
}
fn first_hit(origin: [f64; 3], ray: [f64; 3]) -> Option<f64> {
    let mut edges = Vec::new();
    for (centre, half) in BOXES {
        let (mut lo, mut hi) = (f64::NEG_INFINITY, f64::INFINITY);
        for a in 0..3 {
            if ray[a].abs() < 1e-12 {
                if (origin[a] - centre[a]).abs() > half[a] {
                    hi = f64::NEG_INFINITY;
                }
            } else {
                let p = (centre[a] - half[a] - origin[a]) / ray[a];
                let q = (centre[a] + half[a] - origin[a]) / ray[a];
                lo = lo.max(p.min(q));
                hi = hi.min(p.max(q));
            }
        }
        if hi > lo {
            edges.extend([lo, hi]);
        }
    }
    edges.sort_by(f64::total_cmp);
    edges.windows(2).find_map(|t| {
        let mid = (t[0] + t[1]) * 0.5;
        (t[0] > 0.0 && solid(std::array::from_fn(|a| origin[a] + mid * ray[a]))).then_some(t[0])
    })
}
fn normalise(a: [f64; 3]) -> [f64; 3] {
    let length = a.iter().map(|v| v * v).sum::<f64>().sqrt();
    a.map(|v| v / length)
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn scene() -> (Vec<LevelView>, Vec<Plane<f32>>) {
    let mut views = Vec::new();
    let mut depths = Vec::new();
    for elevation in [-25.0_f64, 25.0] {
        for n in 0..24 {
            let azimuth = (n as f64 * 15.0).to_radians();
            let e = elevation.to_radians();
            let centre = [4.0 * e.cos() * azimuth.cos(), 4.0 * e.cos() * azimuth.sin(), 4.0 * e.sin()];
            let forward = normalise(centre.map(|v| -v));
            let right = normalise(cross(forward, [0.0, 0.0, 1.0]));
            let down = cross(forward, right);
            let rotation = [right, down, forward];
            let translation = rotation.map(|row| -(0..3).map(|a| row[a] * centre[a]).sum::<f64>());
            let camera = Camera {
                rotation: rotation.map(|r| r.map(|v| v as f32)),
                translation: translation.map(|v| v as f32),
                k: [340.0, 330.0, 126.3, 128.7],
            };
            let mut depth = Plane::<f32>::new(256, 256);
            let mut mask = Plane::<u8>::new(256, 256);
            let view = LevelView { width: 256, height: 256, camera, gray: Plane::<f32>::new(256, 256), mask: mask.clone() };
            for y in 0..256 {
                for x in 0..256 {
                    // Independent world ray: parameter is camera Z, NOT unit-ray distance.
                    let ray_camera = [(x as f64 + 0.5 - 126.3) / 340.0, (y as f64 + 0.5 - 128.7) / 330.0, 1.0];
                    let ray = std::array::from_fn(|a| (0..3).map(|b| rotation[b][a] * ray_camera[b]).sum());
                    if let Some(t) = first_hit(centre, ray) {
                        let expected: [f64; 3] = std::array::from_fn(|a| centre[a] + t * ray[a]);
                        let recovered = view.unproject(x, y, t as f32);
                        assert!((0..3).all(|a| (f64::from(recovered[a]) - expected[a]).abs() < 2e-6));
                        let (xy, z) = view.project(expected.map(|v| v as f32));
                        assert!((xy[0] - x as f32).abs() < 0.0002 && (xy[1] - y as f32).abs() < 0.0002);
                        assert!((f64::from(z) - t).abs() < 2e-6);
                        depth.data[y * 256 + x] = t as f32;
                        mask.data[y * 256 + x] = 1;
                    }
                }
            }
            views.push(LevelView { mask, ..view });
            depths.push(depth);
        }
    }
    (views, depths)
}
// Intersect a world-space front-facing line with the exported mesh, independently
// of the engine's renderer. Analytic expected depths: -0.8, -0.6 and -0.98.
fn front_y(triangles: &[stl::Triangle], x: f64) -> f64 {
    triangles
        .iter()
        .filter_map(|tri| {
            let [a, b, c] = tri.map(|p| p.map(f64::from));
            let det = (b[0] - a[0]) * (c[2] - a[2]) - (c[0] - a[0]) * (b[2] - a[2]);
            if det.abs() < 1e-12 {
                return None;
            }
            let u = ((x - a[0]) * (c[2] - a[2]) + a[2] * (c[0] - a[0])) / det;
            let v = (-(b[0] - a[0]) * a[2] - (b[2] - a[2]) * (x - a[0])) / det;
            (u >= -1e-6 && v >= -1e-6 && u + v <= 1.000001).then_some(a[1] + u * (b[1] - a[1]) + v * (c[1] - a[1]))
        })
        .fold(f64::INFINITY, f64::min)
}

#[test]
fn analytic_camera_rays_round_trip_in_production_coordinates() {
    // Runs without a GPU: the oracle ray and intersection are calculated in f64.
    let (views, depths) = scene();
    assert_eq!(views.len(), 48);
    assert!(depths.iter().flat_map(|d| &d.data).filter(|&&d| d > 0.0).count() > 500_000);
}

#[test]
fn exact_depths_preserve_a_recess_and_a_raised_feature() {
    if std::env::var("CRISPMESH_GPU_TESTS").as_deref() != Ok("1") {
        return;
    }
    let retained = std::env::var_os("CRISPMESH_AUDIT_OUTPUT").map(std::path::PathBuf::from);
    let output = retained.clone().unwrap_or_else(|| {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../.local-tools/depth-geometry-audit-{}", std::process::id()))
    });
    assert!(!output.exists(), "audit requires a fresh output directory");
    let (views, depths) = scene();
    let cameras = views.iter().map(|v| v.camera).collect::<Vec<_>>();
    let shape = [96; 3];
    let voxel = 2.0 / 96.0;
    let origin = [-1.0; 3];
    let flags = (0..96 * 96 * 96)
        .map(|i| {
            let p = [i / 9216, i / 96 % 96, i % 96].map(|v| -1.0 + (v as f64 + 0.5) * voxel);
            [0, 2].iter().any(|&b| (0..3).all(|a| (p[a] - BOXES[b].0[a]).abs() < BOXES[b].1[a] + voxel))
        })
        .collect::<Vec<_>>();
    let hull = Hull::from_flags(&flags, shape, origin, voxel);
    let config = DenseConfig { mesh_flat_base: false, ..DenseConfig::default() };
    let gpu = Gpu::new().unwrap();
    let fused = block_on(fusion::tsdf(&gpu, &hull, &cameras, &views, &depths, 0, &config)).unwrap();
    let volume = Volume::from_fused(&hull, fused);
    let report =
        mesh::run_volume(volume, &output, &config, 1, &EventLog::none(), None, 2, &crispmesh_dense::control::Control::none()).unwrap();
    let (triangles, _) = stl::read_binary(&output.join("mesh.stl")).unwrap();
    let measured = [front_y(&triangles, -0.55), front_y(&triangles, -0.3), front_y(&triangles, 0.3)];
    let expected = [-0.8, -0.6, -0.98];
    println!(
        "{}",
        serde_json::json!({"measured_front_y":measured,"analytic_front_y":expected,"voxel":voxel,"closed":report.closed,"genus":report.genus,"triangles":report.triangles})
    );
    for (a, b) in measured.into_iter().zip(expected) {
        assert!((a - b).abs() < 2.0 * voxel, "{a} vs {b}");
    }
    assert!(report.closed);
    if retained.is_none() {
        std::fs::remove_dir_all(output).unwrap();
    }
}

/// Diagnostic rather than an adoption gate: images and cameras never change,
/// only neighbouring prior depths. The selected centre pixel starts at truth.
#[test]
fn report_band_refinement_response_to_neighbour_prior_errors() {
    if std::env::var("CRISPMESH_GPU_TESTS").as_deref() != Ok("1") {
        return;
    }
    let texture =
        |x: f32, y: f32| 0.5 + 0.15 * (0.47 * x + 0.19 * y).sin() + 0.1 * (0.11 * x - 0.37 * y).sin() + 0.08 * (0.91 * x + 0.31 * y).cos();
    let views = [0.0_f32, 0.3, 0.5].map(|baseline| {
        let camera = Camera {
            rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            translation: [-baseline, 0.0, 0.0],
            k: [120.0, 120.0, 64.5, 64.5],
        };
        let gray = Plane {
            width: 128,
            height: 128,
            data: (0..128 * 128).map(|p| texture((p % 128) as f32 + 60.0 * baseline, (p / 128) as f32)).collect(),
        };
        LevelView { width: 128, height: 128, camera, gray, mask: Plane { width: 128, height: 128, data: vec![1; 128 * 128] } }
    });
    let hull = Hull::from_flags(&vec![true; 64 * 64 * 64], [64; 3], [-2.0, -2.0, 1.0], 0.0625);
    let config = DenseConfig::default();
    let gpu = Gpu::new().unwrap();
    let mut matcher = block_on(crispmesh_dense::stereo::matcher::Matcher::new(&gpu, &config, &hull)).unwrap();
    let buffers = matcher.upload(&views);
    let mut results = Vec::new();
    let mut unaggregated_results = Vec::new();
    let mut coherent_results = Vec::new();
    let coherent_config = DenseConfig { coherent_band: true, ..config.clone() };
    let mut coherent_matcher = block_on(crispmesh_dense::stereo::matcher::Matcher::new(&gpu, &coherent_config, &hull)).unwrap();
    let mut independent_results = Vec::new();
    let mut recovered_results = Vec::new();
    let independent = block_on(crispmesh_dense::stereo::planes::Planes::independent(&gpu, &hull)).unwrap();
    let settings = crispmesh_dense::stereo::planes::Settings {
        radius: 5,
        stride: 1,
        best_of: 3,
        min_variance: config.min_variance as f32,
        window_fill: config.window_fill as f32,
        iterations: 6,
        inverse_step: 0.04,
        normal_step: 0.0,
        inverse_range: (0.25, 1.0),
        seed: 0,
    };
    for neighbour_depth in [2.0, 2.12] {
        let mut init = Plane { width: 128, height: 128, data: vec![neighbour_depth; 128 * 128] };
        init.data[64 * 128 + 64] = 2.0;
        let result = block_on(matcher.refine(&buffers, &views, 0, &[1, 2], &init, 0.003, 20, 11, 2.0, 0.55)).unwrap();
        results.push(result.data[64 * 128 + 64]);
        let unaggregated = block_on(matcher.refine(&buffers, &views, 0, &[1, 2], &init, 0.003, 20, 11, 0.0, 0.55)).unwrap();
        unaggregated_results.push(unaggregated.data[64 * 128 + 64]);
        let coherent = block_on(coherent_matcher.refine(&buffers, &views, 0, &[1, 2], &init, 0.003, 20, 11, 2.0, 0.55)).unwrap();
        coherent_results.push(coherent.data[64 * 128 + 64]);
        let planes = crispmesh_dense::stereo::planes::initial_planes(&views[0], &init, &init);
        let (refined, _) = block_on(independent.run(&gpu, &views, 0, &[1, 2], &hull, &planes, &settings)).unwrap();
        independent_results.push(refined[64 * 128 + 64][0]);
        let planes = crispmesh_dense::stereo::planes::initial_planes(&views[0], &result, &init);
        let (recovered, _) = block_on(independent.run(&gpu, &views, 0, &[1, 2], &hull, &planes, &settings)).unwrap();
        recovered_results.push(recovered[64 * 128 + 64][0]);
    }
    assert!((results[0] - 2.0).abs() < 0.01);
    assert!(results.iter().all(|d| d.is_finite() && *d > 0.0));
    assert!(independent_results.iter().all(|d| (*d - 2.0).abs() < 0.01));
    assert!(recovered_results.iter().all(|d| (*d - 2.0).abs() < 0.01));
    assert!(coherent_results.iter().all(|d| (*d - 2.0).abs() < 0.01));
    println!(
        "{}",
        serde_json::json!({"true_plane_depth":2.0,"initial_centre_depth":2.0,"neighbour_prior_depths":[2.0,2.12],"refined_centre_depths":results,"band_without_cost_aggregation":unaggregated_results,"coherent_band_centre_depths":coherent_results,"independent_plane_centre_depths":independent_results,"independent_after_band_centre_depths":recovered_results,"note":"Nonuniform band-prior response is reported, not asserted to be acceptable. Coherent plane scoring is a controlled correction, not a six-object adoption result."})
    );
}
