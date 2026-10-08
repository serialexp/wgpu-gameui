//! Ignored GPU test comparing analytic strokes to Chromium's SVG strokes.
//!
//! The cases (`fixtures/browser/sdf-lines/cases.json`) are drawn by both: the
//! captures are black SVG polylines on a transparent page at DPR 1, 1.5 and 2,
//! so their alpha is coverage. Each case is drawn here the same way and the
//! alpha compared: total coverage, the mean difference over the pixels either
//! paints, and the share of pixels that differ by more than a fifth. Where
//! Chromium's coverage is not exact (`limits`), total coverage is checked
//! against the exact area instead.
//!
//! Every run writes `test_output/stroke_browser_parity/<case>@<dpr>x.png`:
//! Chromium, ours and their difference side by side.
//!
//! Run with `DISPLAY=:0 cargo test --test stroke_browser_parity -- --ignored --nocapture`.

use std::path::{Path, PathBuf};

use image::{GrayImage, Luma};
use serde_json::Value;
use wgpu_gameui::{Affine2, Cap, DrawList, HeadlessGpu, Join, Stroke};

const FIXTURE_ROOT: &str = "fixtures/browser/sdf-lines";
const CROP: f32 = 120.0;
const DPRS: &[(f32, &str)] = &[(1.0, "1"), (1.5, "1.5"), (2.0, "2")];
const ARTIFACTS: &str = "test_output/stroke_browser_parity";

/// How far a case may stray from Chromium.
#[derive(Clone, Copy, Debug)]
struct Limits {
    /// Total coverage, relative.
    mass: f32,
    /// Mean |difference| over painted pixels, in alpha steps of 255.
    mean: f32,
    /// Share of painted pixels more than 51 steps (a fifth) apart.
    far: f32,
    /// The stroke's exact area in CSS px², when total coverage is checked
    /// against it rather than against Chromium's.
    exact_area: Option<f32>,
}

const ORDINARY: Limits = Limits {
    mass: 0.02,
    mean: 6.0,
    far: 0.03,
    exact_area: None,
};

/// The cases where Chromium is not the reference for every measure, and why.
fn limits(id: &str) -> Limits {
    match id {
        // Chromium's coverage of thin diagonal strokes changes with DPR: the
        // 1 px one paints 90, 105 and 94 px² at DPR 1, 1.5 and 2, for an area
        // of 104. Ours keeps the exact area, so the pixels differ more.
        "hairline-diagonal" => Limits {
            mean: 20.0,
            exact_area: Some(90f32.hypot(52.0)),
            ..ORDINARY
        },
        "diagonal-1.5px" => Limits {
            mean: 20.0,
            exact_area: Some(90f32.hypot(55.0) * 1.5),
            ..ORDINARY
        },
        // Chromium's 6 px dots are 2-3% heavier than their exact area.
        "dash-dots" => Limits {
            mean: 10.0,
            exact_area: Some(9.0 * std::f32::consts::PI * 9.0),
            ..ORDINARY
        },
        // Two points a pixel: the line folds back onto itself, and segments
        // that are not neighbours each paint the soft pixels at its edge, so
        // ours is a little heavier at DPR 1 (see docs/design/sdf-lines.md).
        "chart-zigzag" => Limits {
            mass: 0.03,
            mean: 10.0,
            ..ORDINARY
        },
        _ => ORDINARY,
    }
}

/// A case in `cases.json`.
struct Case {
    id: String,
    points: Vec<[f32; 2]>,
    closed: bool,
    stroke: Stroke,
    opacity: f32,
    transform: Option<Affine2>,
}

fn number(value: &Value) -> f32 {
    value.as_f64().expect("a number") as f32
}

fn load_cases() -> Vec<Case> {
    let text = std::fs::read_to_string(format!("{FIXTURE_ROOT}/cases.json")).expect("read cases");
    let cases: Value = serde_json::from_str(&text).expect("parse cases");
    cases
        .as_array()
        .expect("a list of cases")
        .iter()
        .map(|case| {
            let points = case["points"]
                .as_array()
                .expect("points")
                .iter()
                .map(|p| [number(&p[0]), number(&p[1])])
                .collect();
            let cap = match case["cap"].as_str().unwrap_or("butt") {
                "butt" => Cap::Butt,
                "round" => Cap::Round,
                "square" => Cap::Square,
                other => panic!("unknown cap {other}"),
            };
            let join = match case["join"].as_str().unwrap_or("miter") {
                "miter" => Join::Miter,
                "round" => Join::Round,
                "bevel" => Join::Bevel,
                other => panic!("unknown join {other}"),
            };
            let mut stroke = Stroke::new(number(&case["width"])).cap(cap).join(join);
            if let Some(dash) = case["dash"].as_array() {
                stroke = stroke.dashed(number(&dash[0]), number(&dash[1]));
            }
            // SVG's matrix(a b c d e f) maps x' = a·x + c·y + e, y' = b·x + d·y + f.
            let transform = case["transform"].as_array().map(|m| {
                let m: Vec<f32> = m.iter().map(number).collect();
                Affine2::new(m[0], m[2], m[4], m[1], m[3], m[5])
            });
            Case {
                id: case["id"].as_str().expect("id").to_owned(),
                points,
                closed: case["closed"].as_bool().unwrap_or(false),
                stroke,
                opacity: case["opacity"].as_f64().map_or(1.0, |o| o as f32),
                transform,
            }
        })
        .collect()
}

fn draw(list: &mut DrawList, case: &Case) {
    list.push_transform();
    if let Some(m) = case.transform {
        // The draw list composes translations, rotations and scales: a 2x2
        // matrix is R(left) · diag(sx, sy) · R(right) (closed-form SVD).
        let (a, b, c, d) = (m.a, m.c, m.b, m.d);
        let left = 0.5 * (b - c).atan2(a + d) + 0.5 * (b + c).atan2(a - d);
        let right = 0.5 * (b - c).atan2(a + d) - 0.5 * (b + c).atan2(a - d);
        let sx = ((a + d).hypot(b - c) + (a - d).hypot(b + c)) * 0.5;
        let sy = (a * d - b * c) / sx;
        list.translate(m.tx, m.ty);
        list.rotate(left);
        list.scale(sx, sy);
        list.rotate(right);
        let built = list.current_transform();
        for (got, want) in [
            (built.a, m.a),
            (built.b, m.b),
            (built.c, m.c),
            (built.d, m.d),
        ] {
            assert!(
                (got - want).abs() < 1e-5,
                "{}: {built:?} is not {m:?}",
                case.id
            );
        }
    }
    let color = [0.0, 0.0, 0.0, case.opacity];
    if case.closed {
        list.stroke_closed(&case.points, &case.stroke, color);
    } else {
        list.stroke_polyline(&case.points, &case.stroke, color);
    }
    list.pop_transform();
}

#[derive(Debug, Default)]
struct Metrics {
    mass_error: f32,
    mean: f32,
    far: f32,
}

/// `exact_mass`: the expected total alpha when it is not Chromium's.
fn compare(reference: &[u8], ours: &[u8], exact_mass: Option<f32>) -> Metrics {
    let alpha = |pixels: &[u8]| {
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| p[3])
            .collect::<Vec<_>>()
    };
    let (reference, ours) = (alpha(reference), alpha(ours));
    let mass = |values: &[u8]| values.iter().map(|&a| f32::from(a)).sum::<f32>();
    let (expected, actual) = (exact_mass.unwrap_or(mass(&reference)), mass(&ours));
    let mut painted = 0usize;
    let mut total = 0u64;
    let mut far = 0usize;
    for (&e, &a) in reference.iter().zip(&ours) {
        if e == 0 && a == 0 {
            continue;
        }
        painted += 1;
        let delta = e.abs_diff(a);
        total += u64::from(delta);
        if delta > 51 {
            far += 1;
        }
    }
    let painted = painted.max(1) as f32;
    Metrics {
        mass_error: (actual - expected).abs() / expected.max(1.0),
        mean: total as f32 / painted,
        far: far as f32 / painted,
    }
}

/// Chromium, ours and |difference| side by side, as grey alpha.
fn save_side_by_side(path: &Path, size: (u32, u32), reference: &[u8], ours: &[u8]) {
    let (w, h) = size;
    let mut image = GrayImage::from_pixel(w * 3 + 8, h, Luma([128]));
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4 + 3) as usize;
            let (e, a) = (reference[i], ours[i]);
            image.put_pixel(x, y, Luma([255 - e]));
            image.put_pixel(w + 4 + x, y, Luma([255 - a]));
            image.put_pixel(
                2 * w + 8 + x,
                y,
                Luma([255 - e.abs_diff(a).saturating_mul(4)]),
            );
        }
    }
    image.save(path).expect("save side-by-side");
}

#[test]
#[ignore = "requires a GPU adapter; compares Chromium fixtures and writes images"]
fn strokes_match_chromium_svg() {
    let cases = load_cases();
    assert_eq!(
        cases.len(),
        18,
        "cases.json changed: update this test's limits"
    );
    let mut gpu = HeadlessGpu::new().expect("no GPU adapter");
    let artifacts = PathBuf::from(ARTIFACTS);
    if artifacts.exists() {
        std::fs::remove_dir_all(&artifacts).expect("clear old artifacts");
    }
    std::fs::create_dir_all(&artifacts).expect("create artifacts");
    let mut list = gpu.draw_list();
    let mut failures = Vec::new();
    for case in &cases {
        for &(dpr, name) in DPRS {
            let size = ((CROP * dpr) as u32, (CROP * dpr) as u32);
            let path = PathBuf::from(FIXTURE_ROOT)
                .join("captures")
                .join(&case.id)
                .join(format!("alpha@{name}x.png"));
            let reference = image::open(&path)
                .unwrap_or_else(|e| panic!("decode {}: {e}", path.display()))
                .to_rgba8();
            assert_eq!(reference.dimensions(), size, "{}", path.display());
            list.clear();
            draw(&mut list, case);
            let ours = gpu.capture_scaled(&list, size, dpr);
            let limits = limits(&case.id);
            let exact_mass = limits.exact_area.map(|area| area * dpr * dpr * 255.0);
            let metrics = compare(reference.as_raw(), &ours, exact_mass);
            save_side_by_side(
                &artifacts.join(format!("{}@{name}x.png", case.id)),
                size,
                reference.as_raw(),
                &ours,
            );
            println!(
                "{:22} {name:>3}x mass {:6.2}% mean {:5.2} far {:5.2}%",
                case.id,
                metrics.mass_error * 100.0,
                metrics.mean,
                metrics.far * 100.0
            );
            if metrics.mass_error > limits.mass
                || metrics.mean > limits.mean
                || metrics.far > limits.far
            {
                failures.push(format!("{}@{name}x: {metrics:?} over {limits:?}", case.id));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "strokes differ from Chromium:\n{}",
        failures.join("\n")
    );
}
