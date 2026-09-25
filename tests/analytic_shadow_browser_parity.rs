//! Ignored GPU integration test comparing analytic-shadow alpha to Chromium fixtures.
//!
//! Run with `DISPLAY=:0 cargo test --test analytic_shadow_browser_parity -- --ignored --nocapture`.

use std::collections::BTreeSet;
use std::f32::consts::PI;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use image::{ImageBuffer, Rgba, RgbaImage};
use wgpu_gameui::layout::Rect;
use wgpu_gameui::{Affine2, BoxShadow, CornerRadii, HeadlessGpu};

const FIXTURE_ROOT: &str = "fixtures/browser/gpu-chrome-shadows";
const CSS_SIZE: (u32, u32) = (288, 280);
const DPR_CASES: &[(f32, &str)] = &[(1.0, "1"), (1.5, "1.5"), (2.0, "2")];
const ELEMENT: Rect = Rect {
    x: 86.0,
    y: 104.0,
    width: 116.0,
    height: 72.0,
};

#[derive(Clone, Copy)]
struct CssShadow {
    offset: [f32; 2],
    blur: f32,
    spread: f32,
    rgba: [u8; 4],
    inset: bool,
}

impl CssShadow {
    const fn new(x: f32, y: f32, blur: f32, spread: f32, rgba: [u8; 4]) -> Self {
        Self {
            offset: [x, y],
            blur,
            spread,
            rgba,
            inset: false,
        }
    }

    const fn inset(x: f32, y: f32, blur: f32, spread: f32, rgba: [u8; 4]) -> Self {
        Self {
            offset: [x, y],
            blur,
            spread,
            rgba,
            inset: true,
        }
    }

    fn render_value(self) -> BoxShadow {
        // CSS colours are sRGB-encoded, which is the crate's convention too.
        let css = [
            self.rgba[0] as f32 / 255.0,
            self.rgba[1] as f32 / 255.0,
            self.rgba[2] as f32 / 255.0,
            self.rgba[3] as f32 / 255.0,
        ];
        BoxShadow {
            offset: self.offset,
            blur: self.blur,
            spread: self.spread,
            color: css,
            inset: self.inset,
        }
    }
}

const BLACK_70: [u8; 4] = [0, 0, 0, 179];
const BLACK_50: [u8; 4] = [0, 0, 0, 128];
const WHITE_12: [u8; 4] = [255, 255, 255, 31];
const CYAN_65: [u8; 4] = [0, 220, 255, 166];
const B0: CssShadow = CssShadow::new(0.0, 0.0, 0.0, 0.0, BLACK_70);
const B2: CssShadow = CssShadow::new(0.0, 0.0, 2.0, 0.0, BLACK_70);
const B6: CssShadow = CssShadow::new(0.0, 0.0, 6.0, 0.0, BLACK_70);
const B8: CssShadow = CssShadow::new(0.0, 0.0, 8.0, 0.0, BLACK_70);
const B18: CssShadow = CssShadow::new(0.0, 0.0, 18.0, 0.0, BLACK_70);
const B26: CssShadow = CssShadow::new(0.0, 0.0, 26.0, 0.0, BLACK_70);
const B40: CssShadow = CssShadow::new(0.0, 0.0, 40.0, 0.0, BLACK_70);
const B44: CssShadow = CssShadow::new(0.0, 0.0, 44.0, 0.0, BLACK_70);
const TRANSFORM_SHADOW: CssShadow = CssShadow::new(0.0, 10.0, 18.0, 2.0, BLACK_70);

#[derive(Clone, Copy)]
enum Transform {
    None,
    Rotate(f32),
    Scale(f32, f32),
    /// An arbitrary CSS matrix, decomposed into rotation-scale-rotation without
    /// changing the public draw API. Values use CSS matrix(a,b,c,d,0,0) order.
    Matrix(f32, f32, f32, f32),
}

#[derive(Clone, Copy)]
enum ToleranceClass {
    Ordinary,
    Broad,
    Thin,
    Tiny,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ModelContract {
    BrowserParity,
    /// Chromium 153 uses a visibly more compact discrete profile for blur=2 at
    /// DPR 1. The renderer remains accountable to the ideal sigma=blur/2
    /// Gaussian profile instead of treating this browser special case as sigma.
    CompactBrowserBlur2,
}

#[derive(Clone, Copy)]
struct Case {
    id: &'static str,
    rect: Rect,
    radii: CornerRadii,
    shadows: &'static [CssShadow],
    transform: Transform,
    tolerance: ToleranceClass,
}

impl Case {
    fn model(self) -> ModelContract {
        if self.id == "outset-blur-2" {
            ModelContract::CompactBrowserBlur2
        } else {
            ModelContract::BrowserParity
        }
    }
}

const ORDINARY: CornerRadii = CornerRadii::uniform(12.0);
const ASYMMETRIC: CornerRadii = CornerRadii::new(2.0, 18.0, 7.0, 26.0);
const CSS_BORDER_WIDTH: f32 = 1.0;
const MIXED: &[CssShadow] = &[
    CssShadow::new(0.0, 16.0, 40.0, 0.0, BLACK_70),
    CssShadow::new(0.0, 2.0, 6.0, 0.0, BLACK_50),
    CssShadow::inset(0.0, 1.0, 0.0, 0.0, WHITE_12),
    CssShadow::inset(0.0, -1.0, 0.0, 0.0, BLACK_50),
];

const CASES: &[Case] = &[
    Case {
        id: "outset-blur-0",
        rect: ELEMENT,
        radii: CornerRadii::uniform(0.0),
        shadows: &[B0],
        transform: Transform::None,
        tolerance: ToleranceClass::Ordinary,
    },
    Case {
        id: "outset-blur-2",
        rect: ELEMENT,
        radii: CornerRadii::uniform(1.0),
        shadows: &[B2],
        transform: Transform::None,
        tolerance: ToleranceClass::Ordinary,
    },
    Case {
        id: "outset-blur-6",
        rect: ELEMENT,
        radii: ORDINARY,
        shadows: &[B6],
        transform: Transform::None,
        tolerance: ToleranceClass::Ordinary,
    },
    Case {
        id: "outset-blur-8",
        rect: ELEMENT,
        radii: ORDINARY,
        shadows: &[B8],
        transform: Transform::None,
        tolerance: ToleranceClass::Ordinary,
    },
    Case {
        id: "outset-blur-18",
        rect: ELEMENT,
        radii: ORDINARY,
        shadows: &[B18],
        transform: Transform::None,
        tolerance: ToleranceClass::Ordinary,
    },
    Case {
        id: "outset-blur-26",
        rect: ELEMENT,
        radii: ORDINARY,
        shadows: &[B26],
        transform: Transform::None,
        tolerance: ToleranceClass::Broad,
    },
    Case {
        id: "outset-blur-40",
        rect: ELEMENT,
        radii: ORDINARY,
        shadows: &[B40],
        transform: Transform::None,
        tolerance: ToleranceClass::Broad,
    },
    Case {
        id: "outset-blur-44",
        rect: ELEMENT,
        radii: ORDINARY,
        shadows: &[B44],
        transform: Transform::None,
        tolerance: ToleranceClass::Broad,
    },
    Case {
        id: "offset-positive-xy",
        rect: ELEMENT,
        radii: ORDINARY,
        shadows: &[CssShadow::new(13.0, 17.0, 18.0, 0.0, BLACK_70)],
        transform: Transform::None,
        tolerance: ToleranceClass::Ordinary,
    },
    Case {
        id: "offset-negative-xy",
        rect: ELEMENT,
        radii: ORDINARY,
        shadows: &[CssShadow::new(-13.0, -17.0, 18.0, 0.0, BLACK_70)],
        transform: Transform::None,
        tolerance: ToleranceClass::Ordinary,
    },
    Case {
        id: "offset-negative-x",
        rect: ELEMENT,
        radii: ORDINARY,
        shadows: &[CssShadow::new(-18.0, 0.0, 8.0, 0.0, BLACK_70)],
        transform: Transform::None,
        tolerance: ToleranceClass::Ordinary,
    },
    Case {
        id: "offset-negative-y",
        rect: ELEMENT,
        radii: ORDINARY,
        shadows: &[CssShadow::new(0.0, -18.0, 8.0, 0.0, BLACK_70)],
        transform: Transform::None,
        tolerance: ToleranceClass::Ordinary,
    },
    Case {
        id: "spread-negative-asymmetric",
        rect: ELEMENT,
        radii: ASYMMETRIC,
        shadows: &[CssShadow::new(5.0, -4.0, 18.0, -2.0, BLACK_70)],
        transform: Transform::None,
        tolerance: ToleranceClass::Ordinary,
    },
    Case {
        id: "spread-positive-asymmetric",
        rect: ELEMENT,
        radii: ASYMMETRIC,
        shadows: &[CssShadow::new(-5.0, 4.0, 18.0, 4.0, BLACK_70)],
        transform: Transform::None,
        tolerance: ToleranceClass::Ordinary,
    },
    Case {
        id: "inset-basic",
        rect: ELEMENT,
        radii: ORDINARY,
        shadows: &[CssShadow::inset(0.0, 4.0, 8.0, 0.0, BLACK_70)],
        transform: Transform::None,
        tolerance: ToleranceClass::Ordinary,
    },
    Case {
        id: "inset-hole-collapse",
        rect: Rect {
            x: 140.0,
            y: 136.0,
            width: 8.0,
            height: 8.0,
        },
        radii: ORDINARY,
        shadows: &[CssShadow::inset(0.0, 0.0, 18.0, 8.0, BLACK_70)],
        transform: Transform::None,
        tolerance: ToleranceClass::Tiny,
    },
    Case {
        id: "cyan-glow",
        rect: ELEMENT,
        radii: ORDINARY,
        shadows: &[CssShadow::new(0.0, 0.0, 26.0, 4.0, CYAN_65)],
        transform: Transform::None,
        tolerance: ToleranceClass::Broad,
    },
    Case {
        id: "mixed-multi-shadow",
        rect: ELEMENT,
        radii: ORDINARY,
        shadows: MIXED,
        transform: Transform::None,
        tolerance: ToleranceClass::Broad,
    },
    Case {
        id: "thin-blur-40",
        rect: Rect {
            x: 143.0,
            y: 127.0,
            width: 2.0,
            height: 26.0,
        },
        radii: ORDINARY,
        shadows: &[B40],
        transform: Transform::None,
        tolerance: ToleranceClass::Thin,
    },
    Case {
        id: "thin-blur-44",
        rect: Rect {
            x: 143.0,
            y: 127.0,
            width: 2.0,
            height: 26.0,
        },
        radii: ORDINARY,
        shadows: &[B44],
        transform: Transform::None,
        tolerance: ToleranceClass::Thin,
    },
    Case {
        id: "transform-rotate",
        rect: ELEMENT,
        radii: ORDINARY,
        shadows: &[TRANSFORM_SHADOW],
        transform: Transform::Rotate(27.0),
        tolerance: ToleranceClass::Broad,
    },
    Case {
        id: "transform-nonuniform-scale",
        rect: ELEMENT,
        radii: ORDINARY,
        shadows: &[TRANSFORM_SHADOW],
        transform: Transform::Scale(1.35, 0.65),
        tolerance: ToleranceClass::Broad,
    },
    Case {
        id: "transform-skew",
        rect: ELEMENT,
        radii: ORDINARY,
        shadows: &[TRANSFORM_SHADOW],
        transform: Transform::Matrix(1.0, 0.0, 0.445_228_7, 1.0),
        tolerance: ToleranceClass::Broad,
    },
    Case {
        id: "transform-reflect-asymmetric",
        rect: ELEMENT,
        radii: ASYMMETRIC,
        shadows: &[CssShadow::new(8.0, -6.0, 18.0, 4.0, BLACK_70)],
        transform: Transform::Matrix(-1.0, 0.22, 0.18, 1.0),
        tolerance: ToleranceClass::Broad,
    },
];

#[derive(Clone, Copy)]
struct Limits {
    mae: f64,
    p99: u8,
    over_8: f64,
    max_bad_pixels: Option<usize>,
    mass: f64,
    centroid: f64,
}

impl ToleranceClass {
    fn name(self) -> &'static str {
        match self {
            Self::Ordinary => "ordinary",
            Self::Broad => "broad",
            Self::Thin => "thin",
            Self::Tiny => "tiny",
        }
    }
    fn limits(self) -> Limits {
        match self {
            Self::Ordinary => Limits {
                mae: 8.0,
                p99: 48,
                over_8: 0.16,
                max_bad_pixels: None,
                mass: 0.18,
                centroid: 2.0,
            },
            Self::Broad => Limits {
                mae: 10.0,
                p99: 64,
                over_8: 0.22,
                max_bad_pixels: None,
                mass: 0.24,
                centroid: 3.0,
            },
            Self::Thin => Limits {
                mae: 12.0,
                p99: 72,
                over_8: 0.28,
                max_bad_pixels: None,
                mass: 0.30,
                centroid: 4.0,
            },
            // The collapsed inset has only 40--140 affected pixels, making a
            // universal fraction jump by several percentage points per pixel.
            // This absolute budget is predeclared for its fixed 8x8 CSS ROI.
            Self::Tiny => Limits {
                mae: 10.0,
                p99: 64,
                over_8: 1.0,
                // One physical-pixel boundary band around the 8x8 CSS ROI is
                // at most 4*8*2 = 64 pixels at the largest fixture DPR.
                max_bad_pixels: Some(64),
                mass: 0.24,
                centroid: 3.0,
            },
        }
    }
}

#[derive(Default)]
struct Metrics {
    affected: usize,
    mae: f64,
    p99: u8,
    over_8_fraction: f64,
    over_8_count: usize,
    reference_mass: f64,
    gpu_mass: f64,
    mass_relative_error: f64,
    reference_centroid: [f64; 2],
    gpu_centroid: [f64; 2],
    centroid_distance: f64,
    reference_variance: [f64; 2],
    gpu_variance: [f64; 2],
    max_error: u8,
    max_at: [u32; 2],
}

fn transform_linear(transform: Transform) -> Affine2 {
    match transform {
        Transform::None => Affine2::IDENTITY,
        Transform::Rotate(degrees) => Affine2::rotation(degrees * PI / 180.0),
        Transform::Scale(x, y) => Affine2::scale(x, y),
        // CSS matrix(a,b,c,d,e,f) maps (x,y) to (a*x+c*y+e,b*x+d*y+f).
        Transform::Matrix(a, b, c, d) => Affine2::new(a, c, 0.0, b, d, 0.0),
    }
}

fn push_transform(list: &mut wgpu_gameui::DrawList, transform: Transform, rect: Rect) {
    list.push_transform();
    let cx = rect.x + rect.width * 0.5;
    let cy = rect.y + rect.height * 0.5;
    list.translate(cx, cy);
    match transform {
        Transform::None => {}
        Transform::Rotate(degrees) => list.rotate(degrees * PI / 180.0),
        Transform::Scale(x, y) => list.scale(x, y),
        Transform::Matrix(a, b, c, d) => {
            // Closed-form 2x2 SVD. DrawList post-multiplies, so these calls
            // produce R(left) * diag(sx,sy) * R(right), exactly the CSS matrix.
            let left = 0.5 * (b - c).atan2(a + d) + 0.5 * (b + c).atan2(a - d);
            let right = 0.5 * (b - c).atan2(a + d) - 0.5 * (b + c).atan2(a - d);
            let det = a * d - b * c;
            let sx = ((a + d).hypot(b - c) + (a - d).hypot(b + c)) * 0.5;
            let sy = det / sx;
            list.rotate(left);
            list.scale(sx, sy);
            list.rotate(right);
        }
    }
    list.translate(-cx, -cy);

    let expected = Affine2::translation(cx, cy)
        .compose(&transform_linear(transform))
        .compose(&Affine2::translation(-cx, -cy));
    assert_affine_close(list.current_transform(), expected);
}

fn assert_affine_close(actual: Affine2, expected: Affine2) {
    for (name, actual, expected) in [
        ("a", actual.a, expected.a),
        ("b", actual.b, expected.b),
        ("c", actual.c, expected.c),
        ("d", actual.d, expected.d),
        ("tx", actual.tx, expected.tx),
        ("ty", actual.ty, expected.ty),
    ] {
        assert!(
            (actual - expected).abs() <= 5.0e-5,
            "transform {name}: got {actual}, expected {expected}"
        );
    }
}

fn normalized_radii(rect: Rect, radii: CornerRadii) -> CornerRadii {
    let scale = 1.0_f32
        .min(rect.width / (radii.top_left + radii.top_right))
        .min(rect.width / (radii.bottom_left + radii.bottom_right))
        .min(rect.height / (radii.top_left + radii.bottom_left))
        .min(rect.height / (radii.top_right + radii.bottom_right));
    CornerRadii::new(
        radii.top_left * scale,
        radii.top_right * scale,
        radii.bottom_right * scale,
        radii.bottom_left * scale,
    )
}

fn css_padding_box(border_box: Rect, border_radii: CornerRadii) -> (Rect, CornerRadii) {
    let padding_box = Rect::new(
        border_box.x + CSS_BORDER_WIDTH,
        border_box.y + CSS_BORDER_WIDTH,
        border_box.width - 2.0 * CSS_BORDER_WIDTH,
        border_box.height - 2.0 * CSS_BORDER_WIDTH,
    );
    let normalized = normalized_radii(border_box, border_radii);
    let padding_radii = CornerRadii::new(
        (normalized.top_left - CSS_BORDER_WIDTH).max(0.0),
        (normalized.top_right - CSS_BORDER_WIDTH).max(0.0),
        (normalized.bottom_right - CSS_BORDER_WIDTH).max(0.0),
        (normalized.bottom_left - CSS_BORDER_WIDTH).max(0.0),
    );
    (padding_box, padding_radii)
}

fn draw_case(list: &mut wgpu_gameui::DrawList, case: Case) {
    push_transform(list, case.transform, case.rect);
    // CSS paints the first declaration on top, hence the library helpers reverse it.
    let mut rendered = [BoxShadow::default(); 4];
    assert!(case.shadows.len() <= rendered.len());
    for (slot, shadow) in rendered.iter_mut().zip(case.shadows) {
        *slot = shadow.render_value();
    }
    let rendered = &rendered[..case.shadows.len()];
    list.box_shadows_outset(case.rect, case.radii, rendered);
    let (padding_box, padding_radii) = css_padding_box(case.rect, case.radii);
    list.box_shadows_inset(padding_box, padding_radii, rendered);
    list.pop_transform();
}

fn compare(reference: &[u8], gpu: &[u8], width: u32, diffs: &mut Vec<u8>) -> Metrics {
    diffs.clear();
    let mut out = Metrics::default();
    let mut ref_x = 0.0;
    let mut ref_y = 0.0;
    let mut gpu_x = 0.0;
    let mut gpu_y = 0.0;
    let mut ref_x2 = 0.0;
    let mut ref_y2 = 0.0;
    let mut gpu_x2 = 0.0;
    let mut gpu_y2 = 0.0;
    for (index, (expected, actual)) in reference
        .as_chunks::<4>()
        .0
        .iter()
        .zip(gpu.as_chunks::<4>().0.iter())
        .enumerate()
    {
        let e = expected[3];
        let a = actual[3];
        if e == 0 && a == 0 {
            continue;
        }
        let delta = e.abs_diff(a);
        diffs.push(delta);
        out.affected += 1;
        out.mae += delta as f64;
        out.over_8_count += usize::from(delta > 8);
        if delta > out.max_error {
            out.max_error = delta;
            out.max_at = [(index as u32) % width, (index as u32) / width];
        }
        let x = ((index as u32) % width) as f64;
        let y = ((index as u32) / width) as f64;
        out.reference_mass += e as f64;
        out.gpu_mass += a as f64;
        ref_x += x * e as f64;
        ref_y += y * e as f64;
        gpu_x += x * a as f64;
        gpu_y += y * a as f64;
        ref_x2 += x * x * e as f64;
        ref_y2 += y * y * e as f64;
        gpu_x2 += x * x * a as f64;
        gpu_y2 += y * y * a as f64;
    }
    if out.affected != 0 {
        out.mae /= out.affected as f64;
        out.over_8_fraction = out.over_8_count as f64 / out.affected as f64;
        diffs.sort_unstable();
        out.p99 = diffs[((diffs.len() - 1) * 99) / 100];
    }
    if out.reference_mass > 0.0 {
        out.reference_centroid = [ref_x / out.reference_mass, ref_y / out.reference_mass];
        out.reference_variance = [
            ref_x2 / out.reference_mass - out.reference_centroid[0].powi(2),
            ref_y2 / out.reference_mass - out.reference_centroid[1].powi(2),
        ];
    }
    if out.gpu_mass > 0.0 {
        out.gpu_centroid = [gpu_x / out.gpu_mass, gpu_y / out.gpu_mass];
        out.gpu_variance = [
            gpu_x2 / out.gpu_mass - out.gpu_centroid[0].powi(2),
            gpu_y2 / out.gpu_mass - out.gpu_centroid[1].powi(2),
        ];
    }
    out.mass_relative_error = if out.reference_mass > 0.0 {
        (out.gpu_mass - out.reference_mass).abs() / out.reference_mass
    } else if out.gpu_mass == 0.0 {
        0.0
    } else {
        f64::INFINITY
    };
    out.centroid_distance = (out.gpu_centroid[0] - out.reference_centroid[0])
        .hypot(out.gpu_centroid[1] - out.reference_centroid[1]);
    out
}

fn bad_pixels_pass(m: &Metrics, l: Limits) -> bool {
    l.max_bad_pixels
        .map_or(m.over_8_fraction <= l.over_8, |limit| {
            m.over_8_count <= limit
        })
}

fn passes(m: &Metrics, l: Limits) -> bool {
    m.mae <= l.mae
        && m.p99 <= l.p99
        && bad_pixels_pass(m, l)
        && m.mass_relative_error <= l.mass
        && m.centroid_distance <= l.centroid
}

fn passes_except_mass(m: &Metrics, l: Limits) -> bool {
    m.mae <= l.mae && m.p99 <= l.p99 && bad_pixels_pass(m, l) && m.centroid_distance <= l.centroid
}

/// Independent high-resolution integration of an ideal Gaussian-blurred step
/// over the first exterior pixel. `sigma=1` is the authored blur=2 contract.
fn ideal_blur2_exterior_alpha() -> f64 {
    const SAMPLES: usize = 65_536;
    let mut sum = 0.0;
    for i in 0..SAMPLES {
        let distance = (i as f64 + 0.5) / SAMPLES as f64;
        // Numerically integrate the normal CDF tail as a second, independent
        // midpoint integral. This intentionally shares no shader erf code.
        const NORMAL_SAMPLES: usize = 256;
        let upper = 8.0;
        let step = (upper - distance) / NORMAL_SAMPLES as f64;
        let mut tail = 0.0;
        for j in 0..NORMAL_SAMPLES {
            let x = distance + (j as f64 + 0.5) * step;
            tail += (-0.5 * x * x).exp() / (2.0 * std::f64::consts::PI).sqrt() * step;
        }
        sum += tail;
    }
    sum / SAMPLES as f64 * BLACK_70[3] as f64
}

fn json_f64(value: f64) -> String {
    if value.is_finite() {
        format!("{value:.6}")
    } else {
        "null".to_owned()
    }
}

fn load_alpha(path: &Path, expected: (u32, u32), alpha: &mut Vec<u8>) {
    let image = image::open(path)
        .unwrap_or_else(|e| panic!("decode {}: {e}", path.display()))
        .to_rgba8();
    assert_eq!(
        image.dimensions(),
        expected,
        "capture dimension mismatch: {}",
        path.display()
    );
    alpha.clear();
    alpha.extend_from_slice(image.as_raw());
}

fn save_failure_images(
    root: &Path,
    id: &str,
    dpr: &str,
    gpu: &[u8],
    reference: &[u8],
    size: (u32, u32),
) {
    let dir = root.join(id);
    std::fs::create_dir_all(&dir).expect("create shadow artifact directory");
    wgpu_gameui::write_png(dir.join(format!("gpu@{dpr}x.png")), gpu, size)
        .expect("write GPU artifact");
    let mut diff: RgbaImage = ImageBuffer::new(size.0, size.1);
    for ((pixel, expected), actual) in diff
        .pixels_mut()
        .zip(reference.as_chunks::<4>().0.iter())
        .zip(gpu.as_chunks::<4>().0.iter())
    {
        let d = expected[3].abs_diff(actual[3]);
        *pixel = Rgba([d, d, d, 255]);
    }
    diff.save(dir.join(format!("diff@{dpr}x.png")))
        .expect("write diff artifact");
}

fn manifest_ids(text: &str) -> BTreeSet<String> {
    text.lines()
        .filter_map(|line| {
            let marker = "\"id\": \"";
            let start = line.find(marker)? + marker.len();
            let end = line[start..].find('"')? + start;
            Some(line[start..end].to_owned())
        })
        .collect()
}

#[test]
fn ideal_gaussian_oracle_classifies_chromium_blur2_profile() {
    let oracle = ideal_blur2_exterior_alpha();
    assert!(
        (54.0..=58.0).contains(&oracle),
        "independent sigma=1 exterior-pixel oracle changed: {oracle}"
    );
    for (dpr, expected_max) in [("1", 49_u8), ("1.5", 65), ("2", 76)] {
        let path = PathBuf::from(FIXTURE_ROOT)
            .join("captures/outset-blur-2")
            .join(format!("alpha@{dpr}x.png"));
        let image = image::open(&path).unwrap().to_rgba8();
        let scale: f32 = dpr.parse().unwrap();
        let x = (ELEMENT.x * scale).floor() as u32 - 1;
        let y = ((ELEMENT.y + ELEMENT.height * 0.5) * scale).floor() as u32;
        let browser = image.get_pixel(x, y)[3];
        assert!(
            browser <= expected_max,
            "Chromium blur=2 profile is no longer the classified compact model at {dpr}x: {browser}"
        );
    }
}

#[test]
fn css_padding_box_matches_border_box_geometry() {
    let (rect, radii) = css_padding_box(ELEMENT, ORDINARY);
    assert_eq!(rect, Rect::new(87.0, 105.0, 114.0, 70.0));
    assert_eq!(radii, CornerRadii::uniform(11.0));

    let collapsed = Rect::new(140.0, 136.0, 8.0, 8.0);
    let (rect, radii) = css_padding_box(collapsed, ORDINARY);
    assert_eq!(rect, Rect::new(141.0, 137.0, 6.0, 6.0));
    // CSS first normalizes the four 12px outer radii to 4px on the 8x8
    // border box, then subtracts the 1px border for the padding edge.
    assert_eq!(radii, CornerRadii::uniform(3.0));
}

#[test]
fn transparent_source_hides_zero_blur_zero_spread_outset() {
    // Chromium paints an outset shadow behind the source's border box and clips
    // its interior even when capture-alpha makes that source transparent. With
    // zero blur and spread there is consequently no visible alpha to capture.
    for dpr in ["1", "1.5", "2"] {
        let path = PathBuf::from(FIXTURE_ROOT)
            .join("captures/outset-blur-0")
            .join(format!("alpha@{dpr}x.png"));
        let image = image::open(&path)
            .unwrap_or_else(|e| panic!("decode {}: {e}", path.display()))
            .to_rgba8();
        assert!(
            image.pixels().all(|pixel| pixel[3] == 0),
            "{} should contain no visible shadow alpha",
            path.display()
        );
    }
}

#[test]
fn css_transform_mapping_matches_origin_and_matrix_order() {
    for transform in [
        Transform::Rotate(27.0),
        Transform::Scale(1.35, 0.65),
        Transform::Matrix(1.0, 0.0, 0.445_228_7, 1.0),
        Transform::Matrix(-1.0, 0.22, 0.18, 1.0),
    ] {
        let mut list = wgpu_gameui::DrawList::new();
        push_transform(&mut list, transform, ELEMENT);
        let cx = ELEMENT.x + ELEMENT.width * 0.5;
        let cy = ELEMENT.y + ELEMENT.height * 0.5;
        let expected = Affine2::translation(cx, cy)
            .compose(&transform_linear(transform))
            .compose(&Affine2::translation(-cx, -cy));
        assert_affine_close(list.current_transform(), expected);
        let mapped_center = list.current_transform().transform_point([cx, cy]);
        assert!((mapped_center[0] - cx).abs() <= 2.0e-5);
        assert!((mapped_center[1] - cy).abs() <= 2.0e-5);
    }
}

#[test]
#[ignore = "requires a GPU adapter; compares Chromium fixtures and writes diagnostics on failure"]
fn analytic_shadow_alpha_matches_chromium() {
    let manifest = std::fs::read_to_string(format!("{FIXTURE_ROOT}/manifest.json"))
        .expect("read fixture manifest");
    assert!(manifest.contains("\"crop_css_px\": { \"width\": 288, \"height\": 280 }"));
    assert!(manifest.contains("\"device_scale_factors\": [1, 1.5, 2]"));
    assert!(manifest.contains("\"backdrops\": [\"alpha\", \"black\", \"white\"]"));
    let expected_ids: BTreeSet<_> = CASES.iter().map(|case| case.id.to_owned()).collect();
    assert_eq!(
        manifest_ids(&manifest),
        expected_ids,
        "Rust case IDs must exactly match manifest IDs"
    );
    assert_eq!(CASES.len(), 24);
    let capture_ids: BTreeSet<_> = std::fs::read_dir(format!("{FIXTURE_ROOT}/captures"))
        .expect("read capture root")
        .filter_map(|entry| {
            let entry = entry.expect("read capture entry");
            entry
                .file_type()
                .expect("read capture entry type")
                .is_dir()
                .then(|| entry.file_name().to_string_lossy().into_owned())
        })
        .collect();
    assert_eq!(
        capture_ids, expected_ids,
        "capture directory IDs must match manifest IDs"
    );

    let mut gpu = HeadlessGpu::new().expect("no GPU adapter");
    let artifact_root = PathBuf::from("test_output/analytic_shadow_browser_parity");
    if artifact_root.exists() {
        std::fs::remove_dir_all(&artifact_root).expect("clear stale shadow parity artifacts");
    }
    std::fs::create_dir_all(&artifact_root).expect("create shadow parity artifact root");
    let mut failures = Vec::new();
    let mut reference = Vec::new();
    let mut diffs = Vec::new();
    let mut summary = String::from("{\n  \"schema_version\": 2,\n  \"failures\": [\n");
    let mut list = gpu.draw_list();

    for case in CASES {
        for &(dpr, dpr_name) in DPR_CASES {
            let width = (CSS_SIZE.0 as f32 * dpr) as u32;
            let height = (CSS_SIZE.1 as f32 * dpr) as u32;
            let capture_path = PathBuf::from(FIXTURE_ROOT)
                .join("captures")
                .join(case.id)
                .join(format!("alpha@{dpr_name}x.png"));
            load_alpha(&capture_path, (width, height), &mut reference);
            list.clear();
            draw_case(&mut list, *case);
            let pixels = gpu.capture_scaled(&list, (width, height), dpr);
            let metrics = compare(&reference, &pixels, width, &mut diffs);
            let limits = case.tolerance.limits();
            let model_difference =
                case.model() == ModelContract::CompactBrowserBlur2 && dpr_name == "1";
            let classification = if model_difference {
                "browser-model-difference:compact-blur2"
            } else {
                "browser-parity"
            };
            eprintln!(
                "{} @{dpr_name}x [{}; {classification}]: affected={} MAE={:.3} p99={} >8={}/{} ({:.3}%) mass(ref/gpu)={:.0}/{:.0} mass_err={:.3}% centroid(ref/gpu)=({:.2},{:.2})/({:.2},{:.2}) centroid_d={:.3} variance_xy(ref/gpu)=({:.2},{:.2})/({:.2},{:.2}) max={}@{},{}",
                case.id,
                case.tolerance.name(),
                metrics.affected,
                metrics.mae,
                metrics.p99,
                metrics.over_8_count,
                limits.max_bad_pixels.unwrap_or(metrics.affected),
                metrics.over_8_fraction * 100.0,
                metrics.reference_mass,
                metrics.gpu_mass,
                metrics.mass_relative_error * 100.0,
                metrics.reference_centroid[0],
                metrics.reference_centroid[1],
                metrics.gpu_centroid[0],
                metrics.gpu_centroid[1],
                metrics.centroid_distance,
                metrics.reference_variance[0],
                metrics.reference_variance[1],
                metrics.gpu_variance[0],
                metrics.gpu_variance[1],
                metrics.max_error,
                metrics.max_at[0],
                metrics.max_at[1]
            );
            let accepted = if model_difference {
                // Mass is the expected axis of disagreement for the independently
                // classified compact kernel; all spatial/error gates stay active.
                passes_except_mass(&metrics, limits)
            } else {
                passes(&metrics, limits)
            };
            if !accepted {
                save_failure_images(
                    &artifact_root,
                    case.id,
                    dpr_name,
                    &pixels,
                    &reference,
                    (width, height),
                );
                if !failures.is_empty() {
                    summary.push_str(",\n");
                }
                write!(&mut summary, "    {{\"id\":\"{}\",\"dpr\":{},\"class\":\"{}\",\"classification\":\"{}\",\"affected_pixels\":{},\"mae_255\":{},\"p99_255\":{},\"over_8_count\":{},\"over_8_fraction\":{},\"reference_mass_255\":{},\"gpu_mass_255\":{},\"mass_relative_error\":{},\"reference_centroid\":[{},{}],\"gpu_centroid\":[{},{}],\"centroid_distance_px\":{},\"reference_variance_xy\":[{},{}],\"gpu_variance_xy\":[{},{}],\"max_error_255\":{},\"max_at\":[{},{}]}}",
                    case.id, dpr_name, case.tolerance.name(), classification, metrics.affected, json_f64(metrics.mae), metrics.p99, metrics.over_8_count, json_f64(metrics.over_8_fraction), json_f64(metrics.reference_mass), json_f64(metrics.gpu_mass), json_f64(metrics.mass_relative_error), json_f64(metrics.reference_centroid[0]), json_f64(metrics.reference_centroid[1]), json_f64(metrics.gpu_centroid[0]), json_f64(metrics.gpu_centroid[1]), json_f64(metrics.centroid_distance), json_f64(metrics.reference_variance[0]), json_f64(metrics.reference_variance[1]), json_f64(metrics.gpu_variance[0]), json_f64(metrics.gpu_variance[1]), metrics.max_error, metrics.max_at[0], metrics.max_at[1]).unwrap();
                failures.push(format!("{}@{}x", case.id, dpr_name));
            }
        }
    }
    summary.push_str("\n  ],\n  \"result\": \"");
    summary.push_str(if failures.is_empty() { "pass" } else { "fail" });
    summary.push_str("\"\n}\n");
    std::fs::write(artifact_root.join("summary.json"), summary)
        .expect("write shadow parity summary");
    if !failures.is_empty() {
        panic!(
            "{} analytic-shadow parity captures exceeded class tolerances: {} (see {})",
            failures.len(),
            failures.join(", "),
            artifact_root.display()
        );
    }
}

/// Checks how shadow *colour* composites, independent of the (tolerance-bound)
/// shadow shape: for a single-colour shadow, a pixel over an opaque backdrop is
/// `colour·α + backdrop·(1−α)` on sRGB-encoded bytes, with `α` read from the
/// matching alpha capture. The test first confirms Chromium's own black@/white@
/// captures obey that sRGB-space formula, then that our renderer does too
/// (against its own alpha). Blending in linear light instead would be off by up
/// to ~60 levels mid-ramp (black 50% over white: 127 in sRGB, 188 in linear).
#[test]
#[ignore = "requires a GPU adapter; compares Chromium fixtures"]
fn shadow_colour_composites_in_srgb_like_chromium() {
    const TOLERANCE: u8 = 2;
    let single_colour = ["outset-blur-18", "cyan-glow", "offset-positive-xy"];
    let mut gpu = HeadlessGpu::new().expect("no GPU adapter");
    let mut list = gpu.draw_list();
    let (mut alpha, mut on_black, mut on_white) = (Vec::new(), Vec::new(), Vec::new());
    let white = wgpu::Color::WHITE;
    let black = wgpu::Color::BLACK;
    let mut checked = 0usize;

    for case in CASES.iter().filter(|c| single_colour.contains(&c.id)) {
        let colour = case.shadows[0].rgba;
        for &(dpr, dpr_name) in DPR_CASES {
            let size = (
                (CSS_SIZE.0 as f32 * dpr) as u32,
                (CSS_SIZE.1 as f32 * dpr) as u32,
            );
            let dir = PathBuf::from(FIXTURE_ROOT).join("captures").join(case.id);
            load_alpha(
                &dir.join(format!("alpha@{dpr_name}x.png")),
                size,
                &mut alpha,
            );
            load_alpha(
                &dir.join(format!("black@{dpr_name}x.png")),
                size,
                &mut on_black,
            );
            load_alpha(
                &dir.join(format!("white@{dpr_name}x.png")),
                size,
                &mut on_white,
            );
            // The black/white captures keep the source element; only pixels
            // clear of it (plus a 2 CSS px antialiasing margin) are pure shadow.
            let outside = |x: u32, y: u32| {
                let (cx, cy) = (x as f32 / dpr, y as f32 / dpr);
                let r = case.rect;
                cx < r.x - 2.0
                    || cx > r.x + r.width + 2.0
                    || cy < r.y - 2.0
                    || cy > r.y + r.height + 2.0
            };
            let who = format!("{} @{dpr_name}x", case.id);
            checked += check_srgb_over(
                &who, "chromium", size, &outside, colour, &alpha, &on_black, &on_white, TOLERANCE,
            );

            list.clear();
            draw_case(&mut list, *case);
            let ours_alpha = gpu.capture_scaled(&list, size, dpr);
            let ours_black = gpu.capture_scaled_on(&list, size, dpr, black);
            let ours_white = gpu.capture_scaled_on(&list, size, dpr, white);
            checked += check_srgb_over(
                &who,
                "gpu",
                size,
                &outside,
                colour,
                &ours_alpha,
                &ours_black,
                &ours_white,
                TOLERANCE,
            );
        }
    }
    assert!(
        checked > 10_000,
        "too few shadow pixels compared ({checked})"
    );
}

/// Assert `on_black`/`on_white` equal the sRGB-space over-composite of
/// `colour` at the per-pixel coverage in `alpha`, for shadow pixels selected by
/// `outside`. Returns how many pixels were compared.
#[allow(clippy::too_many_arguments)]
fn check_srgb_over(
    who: &str,
    source: &str,
    size: (u32, u32),
    outside: &dyn Fn(u32, u32) -> bool,
    colour: [u8; 4],
    alpha: &[u8],
    on_black: &[u8],
    on_white: &[u8],
    tolerance: u8,
) -> usize {
    let mut compared = 0;
    let mut worst = (0u8, 0u32, 0u32, [0u8; 3], [0u8; 3]);
    for y in 0..size.1 {
        for x in 0..size.0 {
            let i = ((y * size.0 + x) * 4) as usize;
            let a = alpha[i + 3];
            if a == 0 || !outside(x, y) {
                continue;
            }
            compared += 1;
            let a = a as f32 / 255.0;
            for (backdrop, got) in [(0.0, on_black), (255.0, on_white)] {
                let want: [u8; 3] = std::array::from_fn(|c| {
                    (colour[c] as f32 * a + backdrop * (1.0 - a)).round() as u8
                });
                let got = [got[i], got[i + 1], got[i + 2]];
                let off = (0..3).map(|c| got[c].abs_diff(want[c])).max().unwrap();
                if off > worst.0 {
                    worst = (off, x, y, got, want);
                }
            }
        }
    }
    let (off, x, y, got, want) = worst;
    assert!(
        off <= tolerance,
        "{who}: {source} shadow colour is not an sRGB-space composite: at ({x},{y}) got {got:?}, sRGB formula gives {want:?} (off by {off})"
    );
    compared
}
