//! PRIVATE diagnosis (not committed): replays the join of a multi-turn capture from the cached own matches
//! and the per-turn solutions. join_replay MATCHES.json TURN_DIR MASK_DIR RINGS WIDTH HEIGHT FOCAL WORKING OUT.json
use crisp3ds_dense::photos::markers::linalg::{apply, scale, transpose, Camera};
use crisp3ds_dense::photos::ring::{ring_statistics, SceneCamera};
use crisp3ds_dense::photos::solution::Lens;
use crisp3ds_dense::photos::turntable::provider::working_lens;
use crisp3ds_dense::photos::turntable::solver::{join, Matches, Solved};
use serde_json::{json, Value};

fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let read = |p: &str| -> anyhow::Result<Value> { Ok(serde_json::from_slice(&std::fs::read(p)?)?) };
    let mut matches = Matches::from_json(&read(&a[1])?)?;
    let rings: usize = a[4].parse()?;
    let (w, h, f, working): (u32, u32, f64, usize) = (a[5].parse()?, a[6].parse()?, a[7].parse()?, a[8].parse()?);
    let lens = Lens { width: w, height: h, pixels: [f, f, (w as f64 - 1.0) / 2.0, (h as f64 - 1.0) / 2.0], k: [0.0; 3] };
    let wl = working_lens(&lens, working);
    let (sx, sy) = (wl.width as f64 / w as f64, wl.height as f64 / h as f64);
    let count = matches.keypoints.len();
    let per = count / rings;
    // Moving features as in the provider: inside the mask, or matched within their own turn.
    let mut moving: Vec<Vec<bool>> = Vec::new();
    for (n, points) in matches.keypoints.iter().enumerate() {
        let mask = image::open(format!("{}/capture_{:04}.png", a[3], n))?.to_luma8();
        moving.push(
            points
                .iter()
                .map(|p| {
                    let (x, y) = (p[0].floor(), p[1].floor());
                    x >= 0.0
                        && y >= 0.0
                        && (x as u32) < mask.width()
                        && (y as u32) < mask.height()
                        && mask.get_pixel(x as u32, y as u32)[0] > 127
                })
                .collect(),
        );
    }
    for (i, j, list) in &matches.pairs {
        if i / per == j / per {
            for &(p, q) in list {
                moving[*i][p as usize] = true;
                moving[*j][q as usize] = true;
            }
        }
    }
    for (i, j, list) in &mut matches.pairs {
        if *i / per != *j / per {
            list.retain(|&(p, q)| moving[*i][p as usize] && moving[*j][q as usize]);
        }
    }
    matches.keypoints.iter_mut().flatten().for_each(|p| *p = [(p[0] + 0.5) * sx - 0.5, (p[1] + 0.5) * sy - 0.5]);
    let mut solved = Vec::new();
    for r in 0..rings {
        let t = read(&format!("{}/turn-{}.json", a[2], r + 1))?;
        if t["report"].get("unsolved").is_some() {
            solved.push(Solved::unsolved(per, "cached"));
            continue;
        }
        let v = |k: &str| t[k].clone();
        solved.push(Solved {
            rotations: serde_json::from_value(v("rotations"))?,
            translations: serde_json::from_value(v("translations"))?,
            points: serde_json::from_value(v("points"))?,
            observations: serde_json::from_value(v("observations"))?,
            report: v("report"),
        });
    }
    let joined = join(&matches, &Camera::from_lens(&wl), &solved)?;
    for r in 0..rings {
        let cams: Vec<SceneCamera> = (0..per)
            .map(|k| {
                let v = r * per + k;
                let rot = joined.rotations[v];
                SceneCamera {
                    index: k,
                    name: String::new(),
                    rotation: rot,
                    centre: scale(apply(&transpose(&rot), joined.translations[v]), -1.0),
                }
            })
            .collect();
        let s = ring_statistics(&cams, 0.5);
        println!(
            "turn {}: spread {:.2} out {:.2} outward {} miss/dist {}",
            r + 1,
            s["radius_spread_percent"],
            s["out_of_plane_percent"],
            s["cameras_looking_outward"],
            s["optical_axis_miss_of_distance_percent"]["max"]
        );
    }
    println!(
        "{}",
        json!({"reprojection": joined.report["reprojection_px"], "turned": joined.report["turned_onto_axis"], "resected": joined.report["resected"]})
    );
    std::fs::write(
        &a[9],
        serde_json::to_vec(&json!({"rotations": joined.rotations, "translations": joined.translations, "report": joined.report}))?,
    )?;
    Ok(())
}
