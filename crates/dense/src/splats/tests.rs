//! The WebGPU splats against the dense PyTorch reference (`scripts/surface_splats/splat_reference.py`).
//! They need a GPU (`CRISP3DS_GPU_TESTS=1`) and the reference's output (`CRISP3DS_SPLAT_FIXTURE=path.npz`).

use super::*;
use crate::gpu::block_on;
use crate::npz::Npz;

fn fixture() -> Option<Npz> {
    if !crate::gpu::tests_enabled() {
        return None;
    }
    let path = std::env::var("CRISP3DS_SPLAT_FIXTURE").ok()?;
    Some(Npz::read(std::path::Path::new(&path)).expect("fixture"))
}

fn get(npz: &Npz, name: &str) -> Vec<f32> {
    npz.get(name).unwrap_or_else(|e| panic!("{name}: {e}")).to_f32()
}

/// The splats and the camera of a fixture.
pub fn scene(gpu: &Gpu, npz: &Npz) -> (Splats, Camera, u32) {
    let size = npz.get("size").unwrap().to_f64();
    let (width, height, degree) = (size[0] as u32, size[1] as u32, size[2] as u32);
    let (sh0, shn) = (get(npz, "param/sh0"), get(npz, "param/shN"));
    let n = sh0.len() / 3;
    let rest = shn.len() / n;
    let sh: Vec<f32> =
        (0..n).flat_map(|g| sh0[g * 3..g * 3 + 3].iter().chain(&shn[g * rest..(g + 1) * rest]).copied().collect::<Vec<_>>()).collect();
    let rows =
        pack(&get(npz, "param/means"), &get(npz, "param/quats"), &get(npz, "param/scales"), &get(npz, "param/opacities"), &sh).unwrap();
    let splats = Splats::new(gpu, &rows).unwrap();
    let v = get(npz, "viewmat");
    let k = get(npz, "K");
    let camera = Camera {
        rotation: std::array::from_fn(|i| [v[i * 4], v[i * 4 + 1], v[i * 4 + 2]]),
        translation: [v[3], v[7], v[11]],
        intrinsics: [k[0], k[4], k[2], k[5]],
        width,
        height,
    };
    (splats, camera, degree)
}

/// Largest difference relative to the largest reference value, and the number of values off by more than `tolerance`
/// of that scale.
fn compare(ours: &[f32], reference: &[f32], tolerance: f64) -> (f64, usize) {
    assert_eq!(ours.len(), reference.len());
    let scale = reference.iter().fold(0f64, |m, v| m.max(v.abs() as f64)).max(1e-30);
    let largest = ours.iter().zip(reference).fold(0f64, |m, (a, b)| m.max((*a as f64 - *b as f64).abs())) / scale;
    let off = ours.iter().zip(reference).filter(|(a, b)| ((**a as f64) - (**b as f64)).abs() > tolerance * scale).count();
    (largest, off)
}

#[test]
fn rendering_matches_the_reference() {
    let Some(npz) = fixture() else { return };
    let gpu = Gpu::new().expect("GPU");
    let kernels = block_on(Kernels::new(&gpu)).expect("kernels");
    let (splats, camera, degree) = scene(&gpu, &npz);
    let mut frame = Frame::new(&gpu, splats.count, &camera);
    let intersections = block_on(render(&gpu, &kernels, &splats, &camera, degree, &mut frame)).expect("render");
    let n = splats.count as usize;
    let p = (camera.width * camera.height) as usize;
    let projected: Vec<f32> = block_on(gpu.read(&frame.projected, n * PROJECTED as usize)).unwrap();
    let pixels: Vec<f32> = block_on(gpu.read(&frame.pixels, p * PIXEL as usize)).unwrap();
    let column = |k: usize| -> Vec<f32> { (0..n).map(|g| projected[g * PROJECTED as usize + k]).collect() };
    let pixel = |k: usize| -> Vec<f32> { (0..p).map(|i| pixels[i * PIXEL as usize + k]).collect() };
    let mut report: Vec<(&str, (f64, usize))> = Vec::new();
    let means2d: Vec<f32> = (0..n).flat_map(|g| [projected[g * 24], projected[g * 24 + 1]]).collect();
    report.push(("means2d", compare(&means2d, &get(&npz, "means2d"), 1e-4)));
    report.push(("depths", compare(&column(2), &get(&npz, "depths"), 1e-4)));
    let rays: Vec<f32> = (0..n).flat_map(|g| (3..12).map(move |k| (g, k))).map(|(g, k)| projected[g * 24 + k]).collect();
    report.push(("ray_transforms", compare(&rays, &get(&npz, "ray_transforms"), 1e-4)));
    let normals: Vec<f32> = (0..n).flat_map(|g| (12..15).map(move |k| (g, k))).map(|(g, k)| projected[g * 24 + k]).collect();
    report.push(("camera_normals", compare(&normals, &get(&npz, "camera_normals"), 1e-4)));
    let colours: Vec<f32> = (0..n).flat_map(|g| (15..18).map(move |k| (g, k))).map(|(g, k)| projected[g * 24 + k]).collect();
    report.push(("rgb", compare(&colours, &get(&npz, "rgb"), 1e-4)));
    let radii: Vec<f32> = (0..n).flat_map(|g| [projected[g * 24 + 19], projected[g * 24 + 20]]).collect();
    report.push(("radii", compare(&radii, &get(&npz, "radii"), 1e-6)));
    let rendered: Vec<f32> = (0..p).flat_map(|i| (0..3).map(move |k| (i, k))).map(|(i, k)| pixels[i * 16 + k]).collect();
    report.push(("colours", compare(&rendered, &get(&npz, "colours"), 1e-3)));
    let alpha = pixel(3);
    report.push(("alpha", compare(&alpha, &get(&npz, "alpha"), 1e-3)));
    // Normals to the world: R^T n.
    let r = camera.rotation;
    let world: Vec<f32> = (0..p)
        .flat_map(|i| {
            let c = [pixels[i * 16 + 4], pixels[i * 16 + 5], pixels[i * 16 + 6]];
            (0..3).map(move |k| (0..3).map(|j| r[j][k] * c[j]).sum::<f32>())
        })
        .collect();
    report.push(("normals", compare(&world, &get(&npz, "normals"), 1e-3)));
    let expected: Vec<f32> = (0..p).map(|i| pixels[i * 16 + 7] / alpha[i].max(1e-10)).collect();
    report.push(("expected_depth", compare(&expected, &get(&npz, "expected_depth"), 1e-3)));
    report.push(("median", compare(&pixel(9), &get(&npz, "median"), 1e-3)));
    report.push(("distort", compare(&pixel(8), &get(&npz, "distort"), 1e-3)));
    println!("{intersections} intersections");
    let mut failed = Vec::new();
    for (name, (largest, off)) in &report {
        println!("{name:16} largest {largest:.2e}, {off} off");
        if *off > 0 {
            failed.push(*name);
        }
    }
    assert!(failed.is_empty(), "differ from the reference: {failed:?}");
}

#[test]
fn radix_sort_is_stable_and_ordered() {
    if !crate::gpu::tests_enabled() {
        return;
    }
    let gpu = Gpu::new().expect("GPU");
    let kernels = block_on(Kernels::new(&gpu)).expect("kernels");
    let count = 100_003u32;
    let mut state = 12345u64;
    let keys: Vec<u32> = (0..count)
        .map(|_| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((state >> 33) as u32) % 1300
        })
        .collect();
    let sort = SortBuffers::new(&gpu, count);
    gpu.write(&sort.keys[0], &keys);
    gpu.write(&sort.values[0], &(0..count).collect::<Vec<u32>>());
    let at = block_on(radix_sort(&gpu, &kernels, &sort, count, 11)).unwrap();
    let sorted_keys: Vec<u32> = block_on(gpu.read(&sort.keys[at], count as usize)).unwrap();
    let sorted_values: Vec<u32> = block_on(gpu.read(&sort.values[at], count as usize)).unwrap();
    let mut expected: Vec<(u32, u32)> = keys.iter().copied().zip(0..count).collect();
    expected.sort_by_key(|(k, _)| *k);
    for (i, (k, v)) in expected.iter().enumerate() {
        assert_eq!((sorted_keys[i], sorted_values[i]), (*k, *v), "at {i}");
    }
}

/// The backward rasterizer from the reference's image gradients against its gradients of the projected splats.
#[test]
fn raster_backward_matches_the_reference() {
    let Some(npz) = fixture() else { return };
    let gpu = Gpu::new().expect("GPU");
    let kernels = block_on(Kernels::new(&gpu)).expect("kernels");
    let (splats, camera, degree) = scene(&gpu, &npz);
    let mut frame = Frame::new(&gpu, splats.count, &camera);
    block_on(render(&gpu, &kernels, &splats, &camera, degree, &mut frame)).expect("render");
    let p = (camera.width * camera.height) as usize;
    let n = splats.count as usize;
    let (dc, da, dn, dd, dm) = (
        get(&npz, "dimage/colours"),
        get(&npz, "dimage/alpha"),
        get(&npz, "dimage/normals"),
        get(&npz, "dimage/distort"),
        get(&npz, "dimage/median"),
    );
    let r = camera.rotation;
    let mut dpixels = vec![0f32; p * DPIXEL as usize];
    for i in 0..p {
        let o = i * DPIXEL as usize;
        dpixels[o..o + 3].copy_from_slice(&dc[i * 3..i * 3 + 3]);
        dpixels[o + 3] = da[i];
        // World normal = R^T camera normal, so the camera normal's gradient is R times the world normal's.
        for k in 0..3 {
            dpixels[o + 4 + k] = (0..3).map(|j| r[k][j] * dn[i * 3 + j]).sum();
        }
        dpixels[o + 7] = dd[i];
        dpixels[o + 8] = dm[i];
    }
    let dpixels = gpu.upload("dpixels", &dpixels);
    let grads = gpu.zeroed("grads", (n * DSPLAT as usize * 4) as u64);
    block_on(raster_backward(&gpu, &kernels, &camera, &frame, &dpixels, &grads)).expect("backward");
    let ours: Vec<f32> = block_on(gpu.read(&grads, n * DSPLAT as usize)).unwrap();
    let pick = |range: std::ops::Range<usize>| -> Vec<f32> {
        (0..n).flat_map(|g| range.clone().map(move |k| (g, k))).map(|(g, k)| ours[g * DSPLAT as usize + k]).collect()
    };
    let mut failed = Vec::new();
    for (name, values, reference) in [
        ("rgb", pick(0..3), get(&npz, "dsplat/rgb")),
        ("depths", pick(3..4), get(&npz, "dsplat/depths")),
        ("camera_normals", pick(4..7), get(&npz, "dsplat/camera_normals")),
        ("ray_transforms", pick(7..16), get(&npz, "dsplat/ray_transforms")),
        ("means2d", pick(16..18), get(&npz, "dsplat/means2d")),
        ("opacity", pick(18..19), get(&npz, "dsplat/opacity")),
    ] {
        let (largest, off) = compare(&values, &reference, 1e-3);
        println!("{name:16} largest {largest:.2e}, {off} off");
        if off > 0 {
            failed.push(name);
        }
    }
    let params = gpu.zeroed("param grads", (n * PARAMS as usize * 4) as u64);
    block_on(project_backward(&gpu, &kernels, &splats, &camera, degree, &frame, &grads, &params)).expect("projection backward");
    let ours: Vec<f32> = block_on(gpu.read(&params, n * PARAMS as usize)).unwrap();
    let column = |range: std::ops::Range<usize>| -> Vec<f32> {
        (0..n).flat_map(|g| range.clone().map(move |k| (g, k))).map(|(g, k)| ours[g * PARAMS as usize + k]).collect()
    };
    for (name, values, reference) in [
        ("means", column(MEAN..MEAN + 3), get(&npz, "grad/means")),
        ("quats", column(QUAT..QUAT + 4), get(&npz, "grad/quats")),
        ("scales", column(SCALE..SCALE + 3), get(&npz, "grad/scales")),
        ("opacities", column(OPACITY..OPACITY + 1), get(&npz, "grad/opacities")),
        ("sh0", column(SH0..SH0 + 3), get(&npz, "grad/sh0")),
        ("shN", column(SHN..SHN + 45), get(&npz, "grad/shN")),
    ] {
        let (largest, off) = compare(&values, &reference, 1e-3);
        println!("{name:16} largest {largest:.2e}, {off} off");
        if off > 0 {
            failed.push(name);
        }
    }
    assert!(failed.is_empty(), "differ from the reference: {failed:?}");
}

/// The loss's image gradients and terms against the reference's.
#[test]
fn loss_matches_the_reference() {
    let Some(npz) = fixture() else { return };
    let gpu = Gpu::new().expect("GPU");
    let kernels = block_on(Kernels::new(&gpu)).expect("kernels");
    let (splats, camera, degree) = scene(&gpu, &npz);
    let mut frame = Frame::new(&gpu, splats.count, &camera);
    block_on(render(&gpu, &kernels, &splats, &camera, degree, &mut frame)).expect("render");
    let p = (camera.width * camera.height) as usize;
    let (pixels, mask) = (get(&npz, "pixels"), get(&npz, "mask"));
    let rgb: Vec<u8> = pixels.iter().map(|v| (v * 255.0).round() as u8).collect();
    let inside: Vec<bool> = mask.iter().map(|m| *m > 0.5).collect();
    let loss = Loss::new(&gpu, camera.width, camera.height);
    gpu.write(&loss.photo, &pack_photo(&rgb, &inside));
    let bg = get(&npz, "background");
    block_on(loss_backward(&gpu, &kernels, &camera, &frame, &loss, &Weights::default(), [bg[0], bg[1], bg[2]])).expect("loss");
    let dpixels: Vec<f32> = block_on(gpu.read(&loss.dpixels, p * DPIXEL as usize)).unwrap();
    let terms: Vec<f32> = block_on(gpu.read(&loss.terms, p * 5)).unwrap();
    let ssim: Vec<f32> = block_on(gpu.read(&loss.ssim, p)).unwrap();
    let column = |range: std::ops::Range<usize>| -> Vec<f32> {
        (0..p).flat_map(|i| range.clone().map(move |k| (i, k))).map(|(i, k)| dpixels[i * DPIXEL as usize + k]).collect()
    };
    let r = camera.rotation;
    let dn = get(&npz, "dimage/normals");
    let camera_normals: Vec<f32> =
        (0..p).flat_map(|i| (0..3).map(move |k| (i, k))).map(|(i, k)| (0..3).map(|j| r[k][j] * dn[i * 3 + j]).sum::<f32>()).collect();
    let mut failed = Vec::new();
    for (name, values, reference) in [
        ("colours", column(0..3), get(&npz, "dimage/colours")),
        ("alpha", column(3..4), get(&npz, "dimage/alpha")),
        ("normals", column(4..7), camera_normals),
        ("distort", column(7..8), get(&npz, "dimage/distort")),
        ("median", column(8..9), get(&npz, "dimage/median")),
    ] {
        let (largest, off) = compare(&values, &reference, 1e-3);
        println!("{name:16} largest {largest:.2e}, {off} off");
        if off > 0 {
            failed.push(name);
        }
    }
    let mean = |k: usize, d: f32| terms.iter().skip(k).step_by(5).map(|v| *v as f64).sum::<f64>() / (p as f64 * d as f64);
    for (name, ours, reference) in [
        ("l1", mean(0, 3.0), get(&npz, "l1")[0]),
        ("ssim", ssim.iter().map(|v| *v as f64).sum::<f64>() / (3 * p) as f64, get(&npz, "ssim")[0]),
        ("mask_l1", mean(2, 1.0), get(&npz, "mask_l1")[0]),
        ("normal", mean(3, 1.0), get(&npz, "normal")[0]),
        ("distortion", mean(4, 1.0), get(&npz, "distortion")[0]),
    ] {
        let error = (ours - reference as f64).abs() / (reference as f64).abs().max(1e-12);
        println!("{name:16} {ours:.6} reference {reference:.6} ({error:.1e})");
        if error > 1e-4 {
            failed.push(name);
        }
    }
    assert!(failed.is_empty(), "differ from the reference: {failed:?}");
}
