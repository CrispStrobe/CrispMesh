//! The WebGPU network against the MLX prototype: `scripts/neural_surface/export_fixture.py` saves the prototype's
//! parameters, per-sample values, losses and gradients for fixed rays; these tests run the same rays here.
//! They need a GPU (`CRISP3DS_GPU_TESTS=1`) and the fixture (`CRISP3DS_SURFACE_FIXTURE=path.npz`).

use super::*;
use crate::gpu::block_on;
use crate::npz::Npz;

fn fixture() -> Option<Npz> {
    if !crate::gpu::tests_enabled() {
        return None;
    }
    let path = std::env::var("CRISP3DS_SURFACE_FIXTURE").ok()?;
    Some(Npz::read(std::path::Path::new(&path)).expect("fixture"))
}

fn get(npz: &Npz, name: &str) -> Vec<f32> {
    npz.get(name).unwrap_or_else(|e| panic!("{name}: {e}")).to_f32()
}

/// Largest difference relative to the largest reference value.
fn relative(ours: &[f32], reference: &[f32]) -> f64 {
    assert_eq!(ours.len(), reference.len());
    let scale = reference.iter().fold(0f64, |m, v| m.max(v.abs() as f64)).max(1e-30);
    ours.iter().zip(reference).fold(0f64, |m, (a, b)| m.max((*a as f64 - *b as f64).abs())) / scale
}

/// The parameter vector in the order of [`layers`] from a fixture's or checkpoint's named arrays.
pub fn parameter_vector(npz: &Npz, prefix: &str, shape: &Shape) -> Vec<f32> {
    let (layers, count, _) = layers(shape);
    let mut out = vec![0f32; count as usize + 1];
    for (index, layer) in layers.iter().enumerate() {
        for (part, offset, len) in
            [("v", layer.v, layer.inputs * layer.outputs), ("g", layer.g, layer.outputs), ("b", layer.b, layer.outputs)]
        {
            let values = get(npz, &format!("{prefix}{}.{part}", layer_name(index)));
            assert_eq!(values.len(), len as usize, "{}.{part}", layer_name(index));
            out[offset as usize..(offset + len) as usize].copy_from_slice(&values);
        }
    }
    out[count as usize] = get(npz, &format!("{prefix}variance"))[0];
    out
}

#[test]
fn gradients_match_mlx() {
    let Some(npz) = fixture() else { return };
    let gpu = Gpu::new().expect("GPU");
    let kernels = block_on(Kernels::new(&gpu)).expect("kernels");
    let table_log2 = npz.get("table_log2").unwrap().scalar().unwrap() as u32;
    let features = npz.get("features").unwrap().shape[1] as u32;
    let shape = Shape { hidden: npz.get("param/geometry.1.v").unwrap().shape[0] as u32, features, table_log2 };
    let model = Model::new(&gpu, shape, &get(&npz, "param/table"), &parameter_vector(&npz, "param/", &shape)).expect("model");
    let edges = get(&npz, "edges");
    let rays = npz.get("rays_direction").unwrap().shape[0] as u32;
    let per_ray = npz.get("edges").unwrap().shape[1] as u32 - 1;
    let batch = Batch::new(&gpu, &shape, rays, per_ray);
    let (direction, target, mask) = (get(&npz, "rays_direction"), get(&npz, "target"), get(&npz, "mask"));
    let mut ray_data = vec![0f32; rays as usize * 8];
    for r in 0..rays as usize {
        ray_data[r * 8..r * 8 + 3].copy_from_slice(&direction[r * 3..r * 3 + 3]);
        ray_data[r * 8 + 3..r * 8 + 6].copy_from_slice(&target[r * 3..r * 3 + 3]);
        ray_data[r * 8 + 6] = mask[r];
    }
    gpu.write(&batch.ray_data, &ray_data);
    gpu.write(&batch.edges, &edges);
    gpu.write(&batch.points, &get(&npz, "points"));
    let pass = Pass { anneal: npz.get("anneal").unwrap().scalar().unwrap() as f32, step_rays: rays, eikonal: 0.1, mask: 0.1 };
    block_on(async {
        model.begin_step(&gpu, &kernels).await?;
        gradient_pass(&gpu, &kernels, &model, &batch, &pass).await?;
        model.end_gradients(&gpu, &kernels).await
    })
    .expect("gradient pass");

    let samples = batch.samples() as usize;
    let geo = 1 + features as usize;
    let z3: Vec<f32> = block_on(gpu.read(&batch.z3, samples * 4 * geo)).unwrap();
    let distance: Vec<f32> = (0..samples).map(|s| z3[s * 4 * geo]).collect();
    let gradient: Vec<f32> = (0..samples).flat_map(|s| (1..4).map(move |k| (s, k))).map(|(s, k)| z3[(s * 4 + k) * geo]).collect();
    let feature_values: Vec<f32> = (0..samples).flat_map(|s| (1..geo).map(move |c| (s, c))).map(|(s, c)| z3[s * 4 * geo + c]).collect();
    let zc3: Vec<f32> = block_on(gpu.read(&batch.zc3, samples * 3)).unwrap();
    let rgb: Vec<f32> = zc3.iter().map(|z| (1.0 / (1.0 + (-z).exp())) * 1.002 - 0.001).collect();
    let mut report = Vec::new();
    for (name, ours) in [("distance", distance), ("gradient", gradient), ("features", feature_values), ("rgb", rgb)] {
        report.push((name.to_string(), relative(&ours, &get(&npz, name))));
    }
    let losses: Vec<f32> = block_on(gpu.read(&batch.losses, rays as usize * 4)).unwrap();
    let total = |k: usize| losses.iter().skip(k).step_by(4).map(|v| *v as f64).sum::<f64>();
    let colour_loss = total(0) / (3.0 * rays as f64);
    let eikonal = total(1) / samples as f64;
    let mask_loss = total(2) / rays as f64;
    for (name, ours) in [("colour_loss", colour_loss), ("eikonal", eikonal), ("mask_loss", mask_loss)] {
        report.push((name.to_string(), relative(&[ours as f32], &get(&npz, name))));
    }
    let table_grad: Vec<f32> = block_on(gpu.read(&model.table_grad, shape.table_floats() as usize)).unwrap();
    report.push(("grad/table".into(), relative(&table_grad, &get(&npz, "grad/table"))));
    let grads: Vec<f32> = block_on(gpu.read(&model.parameters_grad, model.variance as usize + 1)).unwrap();
    for (index, layer) in model.layers.iter().enumerate() {
        for (part, offset, len) in
            [("v", layer.v, layer.inputs * layer.outputs), ("g", layer.g, layer.outputs), ("b", layer.b, layer.outputs)]
        {
            let name = format!("grad/{}.{part}", layer_name(index));
            report.push((name.clone(), relative(&grads[offset as usize..(offset + len) as usize], &get(&npz, &name))));
        }
    }
    report.push(("grad/variance".into(), relative(&grads[model.variance as usize..], &get(&npz, "grad/variance"))));
    let mut failed = Vec::new();
    for (name, error) in &report {
        println!("{name:24} {error:.2e}");
        if *error > 2e-3 {
            failed.push(name.clone());
        }
    }
    assert!(failed.is_empty(), "differ from MLX: {failed:?}");
}

#[test]
fn sampler_matches_mlx() {
    let Some(npz) = fixture() else { return };
    let gpu = Gpu::new().expect("GPU");
    let kernels = block_on(Kernels::new(&gpu)).expect("kernels");
    let table_log2 = npz.get("table_log2").unwrap().scalar().unwrap() as u32;
    let features = npz.get("features").unwrap().shape[1] as u32;
    let shape = Shape { hidden: npz.get("param/geometry.1.v").unwrap().shape[0] as u32, features, table_log2 };
    let model = Model::new(&gpu, shape, &get(&npz, "param/table"), &parameter_vector(&npz, "param/", &shape)).expect("model");
    let reference = get(&npz, "sampled_edges");
    let rays = npz.get("sampled_edges").unwrap().shape[0] as u32;
    let sampling = Sampling { coarse: 32, added: 8, rounds: 4, band: 0.0 };
    let batch = Batch::new(&gpu, &shape, rays, sampling.per_ray());
    let (origin, direction, offsets) = (get(&npz, "rays_origin"), get(&npz, "rays_direction"), get(&npz, "sample_offsets"));
    let mut origins = vec![0f32; rays as usize * 8];
    let mut ray_data = vec![0f32; rays as usize * 8];
    for r in 0..rays as usize {
        origins[r * 8..r * 8 + 3].copy_from_slice(&origin[r * 3..r * 3 + 3]);
        origins[r * 8 + 3] = offsets[r];
        ray_data[r * 8..r * 8 + 3].copy_from_slice(&direction[r * 3..r * 3 + 3]);
    }
    gpu.write(&batch.origins, &origins);
    gpu.write(&batch.ray_data, &ray_data);
    block_on(async {
        model.begin_step(&gpu, &kernels).await?;
        sample(&gpu, &kernels, &model, &batch, &sampling, None).await
    })
    .expect("sampler");
    let edges: Vec<f32> = block_on(gpu.read(&batch.edges, reference.len())).unwrap();
    let differences: Vec<f32> = edges.iter().zip(&reference).map(|(a, b)| (a - b).abs()).collect();
    let largest = differences.iter().fold(0f32, |m, v| m.max(*v));
    let off = differences.iter().filter(|v| **v > 1e-4).count();
    println!("sampler: largest difference {largest:.2e}, {off} of {} edges differ by more than 1e-4", reference.len());
    // At sharpness 512 a rounding difference of 1e-7 in a distance moves a quantile by ~1e-4 of the ray; an edge
    // drawn from another interval would be a sample spacing (~0.06) away.
    assert!(largest < 2e-3, "largest edge difference {largest}");
}

#[test]
fn nearest_points_are_exact() {
    let mut random = train::Random::new(3);
    let points: Vec<[f64; 3]> = (0..2000).map(|_| std::array::from_fn(|_| random.normal() * 0.2)).collect();
    let tree = train::Tree::new(&points);
    for n in 0..500 {
        // Near the points and far away (where pruning by the last split alone visits almost everything).
        let spread = if n % 2 == 0 { 0.3 } else { 3.0 };
        let q: [f64; 3] = std::array::from_fn(|_| (random.uniform() * 2.0 - 1.0) * spread);
        let d2 = |p: &[f64; 3]| (0..3).map(|i| (p[i] - q[i]) * (p[i] - q[i])).sum::<f64>();
        let brute = points.iter().map(d2).fold(f64::INFINITY, f64::min);
        let (index, found) = tree.nearest(q);
        assert_eq!(found, brute);
        assert_eq!(d2(&points[index]), brute);
    }
}

/// The matrix product in its three modes against a CPU product, with its speed (`CRISP3DS_GPU_TESTS=1`).
#[test]
fn matrix_products_are_exact_and_fast() {
    if !crate::gpu::tests_enabled() {
        return;
    }
    let gpu = Gpu::new().expect("GPU");
    let kernels = block_on(Kernels::new(&gpu)).expect("kernels");
    let mut random = train::Random::new(5);
    let (m, n, k) = (65536u32, 71u32, 64u32);
    let a: Vec<f32> = (0..m * k).map(|_| random.normal() as f32).collect();
    let w: Vec<f32> = (0..n * k).map(|_| random.normal() as f32).collect();
    let bias: Vec<f32> = (0..n).map(|_| random.normal() as f32).collect();
    let x: Vec<f32> = (0..m * n).map(|_| random.normal() as f32).collect();
    let (ab, wb, biasb, xb) = (gpu.upload("a", &a), gpu.upload("w", &w), gpu.upload("bias", &bias), gpu.upload("x", &x));
    let out = gpu.zeroed("c", m as u64 * n.max(k) as u64 * 4);
    let partial = gpu.zeroed("partial", m.div_ceil(CHUNK) as u64 * (n * k) as u64 * 4);
    let flops = 2.0 * m as f64 * n as f64 * k as f64;
    // Forward: [m x k] [n x k]^T + bias on every 4th row.
    let p = MatmulParams { bias_stride: 4, ..mm(m, n, k, 0) };
    let time = |f: &dyn Fn()| {
        f();
        let _: Vec<f32> = block_on(gpu.read(&out, 1)).unwrap();
        let start = web_time::Instant::now();
        for _ in 0..10 {
            f();
        }
        let _: Vec<f32> = block_on(gpu.read(&out, 1)).unwrap();
        start.elapsed().as_secs_f64() / 10.0
    };
    let seconds = time(&|| block_on(matmul(&gpu, &kernels, p, &ab, &wb, &out, &biasb)).unwrap());
    let c: Vec<f32> = block_on(gpu.read(&out, (m * n) as usize)).unwrap();
    for row in (0..m as usize).step_by(997) {
        for col in 0..n as usize {
            let mut expected: f64 = (0..k as usize).map(|i| a[row * k as usize + i] as f64 * w[col * k as usize + i] as f64).sum();
            if row % 4 == 0 {
                expected += bias[col] as f64;
            }
            assert!((c[row * n as usize + col] as f64 - expected).abs() < 1e-3, "forward {row} {col}");
        }
    }
    println!("forward {m}x{n}x{k}: {:.2} ms, {:.0} GFLOP/s", seconds * 1e3, flops / seconds / 1e9);
    // Input gradient: [m x n] [n x k] (x times the weights as an n x k matrix).
    let p = mm(m, k, n, 1);
    let seconds = time(&|| block_on(matmul(&gpu, &kernels, p, &xb, &wb, &out, &biasb)).unwrap());
    let c: Vec<f32> = block_on(gpu.read(&out, (m * k) as usize)).unwrap();
    for row in (0..m as usize).step_by(997) {
        for col in 0..k as usize {
            let expected: f64 = (0..n as usize).map(|i| x[row * n as usize + i] as f64 * w[i * k as usize + col] as f64).sum();
            assert!((c[row * k as usize + col] as f64 - expected).abs() < 1e-3, "input {row} {col}");
        }
    }
    println!("input gradient {m}x{k}x{n}: {:.2} ms, {:.0} GFLOP/s", seconds * 1e3, flops / seconds / 1e9);
    // Weight gradient: sum over rows of x^T a, per chunk of rows.
    let p = mm(n, k, m, 2);
    let seconds = time(&|| block_on(matmul(&gpu, &kernels, p, &xb, &ab, &partial, &biasb)).unwrap());
    let chunks = m.div_ceil(CHUNK) as usize;
    let parts: Vec<f32> = block_on(gpu.read(&partial, chunks * (n * k) as usize)).unwrap();
    for (row, col) in [(0usize, 0usize), (5, 17), (n as usize - 1, k as usize - 1), (33, 40)] {
        let got: f64 = (0..chunks).map(|c| parts[(c * n as usize + row) * k as usize + col] as f64).sum();
        let expected: f64 = (0..m as usize).map(|r| x[r * n as usize + row] as f64 * a[r * k as usize + col] as f64).sum();
        assert!((got - expected).abs() < 1e-2 * (1.0 + expected.abs()), "weight {row} {col}: {got} {expected}");
    }
    println!("weight gradient {n}x{k} over {m}: {:.2} ms, {:.0} GFLOP/s", seconds * 1e3, flops / seconds / 1e9);
}

/// The band-limited sampler on the prototype's own hull grid.
#[test]
fn band_sampler_matches_mlx() {
    let Some(npz) = fixture() else { return };
    let gpu = Gpu::new().expect("GPU");
    let kernels = block_on(Kernels::new(&gpu)).expect("kernels");
    let table_log2 = npz.get("table_log2").unwrap().scalar().unwrap() as u32;
    let features = npz.get("features").unwrap().shape[1] as u32;
    let shape = Shape { hidden: npz.get("param/geometry.1.v").unwrap().shape[0] as u32, features, table_log2 };
    let model = Model::new(&gpu, shape, &get(&npz, "param/table"), &parameter_vector(&npz, "param/", &shape)).expect("model");
    let reference = get(&npz, "band_edges");
    let rays = npz.get("band_edges").unwrap().shape[0] as u32;
    let sampling = Sampling { coarse: 32, added: 8, rounds: 4, band: 0.05 };
    let batch = Batch::new(&gpu, &shape, rays, sampling.per_ray());
    let (origin, direction, offsets) = (get(&npz, "rays_origin"), get(&npz, "rays_direction"), get(&npz, "band_offsets"));
    let mut origins = vec![0f32; rays as usize * 8];
    let mut ray_data = vec![0f32; rays as usize * 8];
    for r in 0..rays as usize {
        origins[r * 8..r * 8 + 3].copy_from_slice(&origin[r * 3..r * 3 + 3]);
        origins[r * 8 + 3] = offsets[r];
        ray_data[r * 8..r * 8 + 3].copy_from_slice(&direction[r * 3..r * 3 + 3]);
    }
    gpu.write(&batch.origins, &origins);
    gpu.write(&batch.ray_data, &ray_data);
    let hull = Hull { grid: gpu.upload("hull", &get(&npz, "hull_grid")), resolution: npz.get("hull_grid").unwrap().shape[0] as u32 };
    block_on(async {
        model.begin_step(&gpu, &kernels).await?;
        sample(&gpu, &kernels, &model, &batch, &sampling, Some(&hull)).await
    })
    .expect("sampler");
    let edges: Vec<f32> = block_on(gpu.read(&batch.edges, reference.len())).unwrap();
    let largest = edges.iter().zip(&reference).fold(0f32, |m, (a, b)| m.max((a - b).abs()));
    let off = edges.iter().zip(&reference).filter(|(a, b)| (*a - *b).abs() > 1e-4).count();
    println!("band sampler: largest difference {largest:.2e}, {off} of {} edges differ by more than 1e-4", reference.len());
    // Besides the quantile shifts of `sampler_matches_mlx`, a ray's band limit can land one of 256 steps apart
    // where the hull's distance is at the threshold.
    assert!(largest < 1e-2 && off * 50 <= reference.len(), "largest edge difference {largest}, {off} edges");
}

/// Our hull grid against the prototype's (needs the pack the fixture was made from: CRISP3DS_SURFACE_PACK).
#[test]
fn hull_grid_matches_mlx() {
    let Some(npz) = fixture() else { return };
    let Ok(pack) = std::env::var("CRISP3DS_SURFACE_PACK") else { return };
    let scene = train::load_pack(std::path::Path::new(&pack)).expect("pack");
    let reference = get(&npz, "hull_grid");
    let ours = train::hull_grid(&scene, 160);
    let largest = ours.iter().zip(&reference).fold(0f32, |m, (a, b)| m.max((a - b).abs()));
    let mean = ours.iter().zip(&reference).map(|(a, b)| (a - b).abs() as f64).sum::<f64>() / ours.len() as f64;
    println!("hull grid: largest difference {largest:.2e}, mean {mean:.2e}");
    assert!(largest < 1e-4, "hull grid differs by {largest}");
}

/// Two training steps (rays in two parts, gradients accumulated, Adam without bias correction) against MLX.
#[test]
fn training_steps_match_mlx() {
    let Some(npz) = fixture() else { return };
    let gpu = Gpu::new().expect("GPU");
    let kernels = block_on(Kernels::new(&gpu)).expect("kernels");
    let table_log2 = npz.get("table_log2").unwrap().scalar().unwrap() as u32;
    let features = npz.get("features").unwrap().shape[1] as u32;
    let shape = Shape { hidden: npz.get("param/geometry.1.v").unwrap().shape[0] as u32, features, table_log2 };
    let before_table = get(&npz, "param/table");
    let before = parameter_vector(&npz, "param/", &shape);
    let model = Model::new(&gpu, shape, &before_table, &before).expect("model");
    let rays = npz.get("rays_direction").unwrap().shape[0];
    let per_ray = npz.get("edges").unwrap().shape[1] - 1;
    let half = rays / 2;
    let batch = Batch::new(&gpu, &shape, half as u32, per_ray as u32);
    let (direction, target, mask, edges, points) =
        (get(&npz, "rays_direction"), get(&npz, "target"), get(&npz, "mask"), get(&npz, "edges"), get(&npz, "points"));
    let pass = Pass { anneal: npz.get("anneal").unwrap().scalar().unwrap() as f32, step_rays: rays as u32, eikonal: 0.1, mask: 0.1 };
    let mut failed = Vec::new();
    for step in 0..2u32 {
        block_on(async {
            model.begin_step(&gpu, &kernels).await?;
            for part in 0..2 {
                let range = part * half..(part + 1) * half;
                let mut ray_data = vec![0f32; half * 8];
                for (i, r) in range.clone().enumerate() {
                    ray_data[i * 8..i * 8 + 3].copy_from_slice(&direction[r * 3..r * 3 + 3]);
                    ray_data[i * 8 + 3..i * 8 + 6].copy_from_slice(&target[r * 3..r * 3 + 3]);
                    ray_data[i * 8 + 6] = mask[r];
                }
                gpu.write(&batch.ray_data, &ray_data);
                gpu.write(&batch.edges, &edges[range.start * (per_ray + 1)..range.end * (per_ray + 1)]);
                gpu.write(&batch.points, &points[range.start * per_ray * 3..range.end * per_ray * 3]);
                gradient_pass(&gpu, &kernels, &model, &batch, &pass).await?;
            }
            model.end_gradients(&gpu, &kernels).await?;
            model.adam(&gpu, &kernels, 5e-4, step, false).await
        })
        .expect("step");
        let prefix = format!("step{}/", step + 1);
        let table: Vec<f32> = block_on(gpu.read(&model.table, before_table.len())).unwrap();
        let parameters: Vec<f32> = block_on(gpu.read(&model.parameters, before.len())).unwrap();
        let mut compare = |name: &str, ours: &[f32], start: &[f32], reference: &[f32]| {
            // Changes from the start; Adam moves each value by about the learning rate times 3 in the first steps.
            let typical = reference.iter().zip(start).map(|(r, s)| (r - s).abs()).fold(0f32, f32::max).max(1e-12);
            let off = ours.iter().zip(reference).filter(|(a, b)| (*a - *b).abs() > 0.05 * typical).count();
            let fraction = off as f64 / ours.len() as f64;
            println!("{prefix}{name:22} {off} of {} differ by more than 5 % of the largest change", ours.len());
            if fraction > 0.01 {
                failed.push(format!("{prefix}{name}"));
            }
        };
        compare("table", &table, &before_table, &get(&npz, &format!("{prefix}table")));
        let reference = parameter_vector(&npz, &prefix, &shape);
        for (index, layer) in model.layers.iter().enumerate() {
            for (part, offset, len) in
                [("v", layer.v, layer.inputs * layer.outputs), ("g", layer.g, layer.outputs), ("b", layer.b, layer.outputs)]
            {
                let r = offset as usize..(offset + len) as usize;
                compare(&format!("{}.{part}", layer_name(index)), &parameters[r.clone()], &before[r.clone()], &reference[r]);
            }
        }
        let v = model.variance as usize;
        compare("variance", &parameters[v..], &before[v..], &reference[v..]);
    }
    assert!(failed.is_empty(), "differ from MLX: {failed:?}");
}

/// A scene made from an inputs directory against the pack `pack_scene.py` made from the same files
/// (CRISP3DS_SURFACE_INPUTS, _MASKS, _HULL and _INPUTS_PACK; the pack's photos are JPEG, its hull samples random).
#[test]
fn scene_from_inputs_matches_the_pack() {
    let var = |name: &str| std::env::var(name).ok().map(std::path::PathBuf::from);
    let (Some(inputs), Some(masks), Some(hull), Some(pack)) =
        (var("CRISP3DS_SURFACE_INPUTS"), var("CRISP3DS_SURFACE_MASKS"), var("CRISP3DS_SURFACE_HULL"), var("CRISP3DS_SURFACE_INPUTS_PACK"))
    else {
        return;
    };
    let ours = train::scene_from_inputs(&inputs, var("CRISP3DS_SURFACE_PHOTOS").as_deref(), Some(&masks), &hull, 2, 0).expect("scene");
    let theirs = train::load_pack(&pack).expect("pack");
    assert_eq!(ours.views.len(), theirs.views.len());
    let radius = |s: &train::Scene| 1.0 / (1.15 * s.unit);
    let shift = (0..3).map(|i| (ours.centre[i] - theirs.centre[i]).powi(2)).sum::<f64>().sqrt();
    println!("centre shift {:.2e} of the radius, radius ratio {:.5}", shift / radius(&theirs), radius(&ours) / radius(&theirs));
    assert!(shift < 0.01 * radius(&theirs) && (radius(&ours) / radius(&theirs) - 1.0).abs() < 0.01);
    let (mut mask_off, mut pixels, mut colour) = (0usize, 0usize, 0f64);
    for (a, b) in ours.views.iter().zip(&theirs.views) {
        assert_eq!((a.width, a.height), (b.width, b.height));
        for i in 0..4 {
            assert!((a.intrinsics[i] - b.intrinsics[i]).abs() < 1e-9, "intrinsics {i}: {} {}", a.intrinsics[i], b.intrinsics[i]);
        }
        for p in 0..a.mask.len() {
            pixels += 1;
            if a.mask[p] != b.mask[p] {
                mask_off += 1;
            } else if a.mask[p] {
                colour += (0..3).map(|k| (a.image[p * 3 + k] as f64 - b.image[p * 3 + k] as f64).abs()).sum::<f64>() / 3.0;
            }
        }
    }
    let inside = theirs.views.iter().map(|v| v.mask.iter().filter(|m| **m).count()).sum::<usize>();
    println!(
        "masks differ on {mask_off} of {pixels} pixels; mean colour difference inside {:.2} (JPEG in the pack)",
        colour / inside as f64
    );
    assert!(mask_off * 1000 < pixels && colour / (inside as f64) < 3.0);
}
