//! Deterministic calibration of the fixed y quadrature in `render/ui.wgsl`.
//!
//! The shader integrates rounded-rectangle coverage analytically in x and uses
//! four midpoint strata in y. Gauss-Legendre is usually superior for smooth
//! integrands, while midpoint has simpler nodes and equal weights in the shader.
//! This test records the whole-grid tradeoff rather than forcing midpoint to win;
//! eight-point results also show what increasing the shader's sample count buys.

use std::f64::consts::PI;

const GRID_SIDE: usize = 41;
const REFERENCE_STRATA: usize = 4096;
const GL4_NODES: [f64; 4] = [
    -0.861_136_311_594_052_6,
    -0.339_981_043_584_856_3,
    0.339_981_043_584_856_3,
    0.861_136_311_594_052_6,
];
const GL4_WEIGHTS: [f64; 4] = [
    0.347_854_845_137_453_8,
    0.652_145_154_862_546_1,
    0.652_145_154_862_546_1,
    0.347_854_845_137_453_8,
];
const GL8_NODES: [f64; 8] = [
    -0.960_289_856_497_536_3,
    -0.796_666_477_413_626_7,
    -0.525_532_409_916_329,
    -0.183_434_642_495_649_8,
    0.183_434_642_495_649_8,
    0.525_532_409_916_329,
    0.796_666_477_413_626_7,
    0.960_289_856_497_536_3,
];
const GL8_WEIGHTS: [f64; 8] = [
    0.101_228_536_290_376_3,
    0.222_381_034_453_374_5,
    0.313_706_645_877_887_3,
    0.362_683_783_378_362,
    0.362_683_783_378_362,
    0.313_706_645_877_887_3,
    0.222_381_034_453_374_5,
    0.101_228_536_290_376_3,
];

#[derive(Clone, Copy)]
struct Case {
    name: &'static str,
    size: [f64; 2],
    // TL, TR, BR, BL, after CSS radius normalization.
    radii: [f64; 4],
    blur: f64,
}

const CASES: [Case; 6] = [
    Case {
        name: "thin-2x26-blur-40",
        size: [2.0, 26.0],
        radii: [1.0; 4],
        blur: 40.0,
    },
    Case {
        name: "thin-2x26-blur-44",
        size: [2.0, 26.0],
        radii: [1.0; 4],
        blur: 44.0,
    },
    Case {
        name: "broad-116x72-blur-40",
        size: [116.0, 72.0],
        radii: [12.0; 4],
        blur: 40.0,
    },
    Case {
        name: "broad-116x72-blur-44",
        size: [116.0, 72.0],
        radii: [12.0; 4],
        blur: 44.0,
    },
    Case {
        name: "contact-26x26-blur-40",
        size: [26.0, 26.0],
        radii: [12.0; 4],
        blur: 40.0,
    },
    Case {
        name: "asymmetric-contact-blur-40",
        size: [52.0, 34.0],
        radii: [2.0, 17.0, 7.0, 13.0],
        blur: 40.0,
    },
];

#[derive(Clone, Copy, Default)]
struct ErrorStats {
    absolute_sum: f64,
    maximum: f64,
    samples: usize,
}

impl ErrorStats {
    fn record(&mut self, actual: f64, reference: f64) {
        let error = (actual - reference).abs();
        self.absolute_sum += error;
        self.maximum = self.maximum.max(error);
        self.samples += 1;
    }

    fn mean(self) -> f64 {
        self.absolute_sum / self.samples as f64
    }
}

// This is the scalar equivalent of the shader's `shadow_erf`; retaining its
// approximation in both candidate and reference isolates y-quadrature error.
fn shader_erf(value: f64) -> f64 {
    let sign = value.signum();
    let a = value.abs();
    let r1 = 1.0 + (0.278393 + (0.230389 + (0.000972 + 0.078108 * a) * a) * a) * a;
    sign - sign / r1.powi(4)
}

fn gaussian(x: f64, sigma: f64) -> f64 {
    (-x * x / (2.0 * sigma * sigma)).exp() / ((2.0 * PI).sqrt() * sigma)
}

fn side_extent(y: f64, radius: f64, half: [f64; 2]) -> f64 {
    let delta = (half[1] - radius - y.abs()).min(0.0);
    half[0] - radius + (radius * radius - delta * delta).max(0.0).sqrt()
}

fn analytic_x(x: f64, source_y: f64, sigma: f64, radii: [f64; 4], half: [f64; 2]) -> f64 {
    let (left_radius, right_radius) = if source_y < 0.0 {
        (radii[0], radii[1])
    } else {
        (radii[3], radii[2])
    };
    let left = side_extent(source_y, left_radius, half);
    let right = side_extent(source_y, right_radius, half);
    let scale = std::f64::consts::FRAC_1_SQRT_2 / sigma;
    0.5 * (shader_erf((x + left) * scale) - shader_erf((x - right) * scale))
}

fn bounds(point_y: f64, half_y: f64, sigma: f64) -> (f64, f64) {
    let low = point_y - half_y;
    let high = point_y + half_y;
    (
        (-3.0 * sigma).max(low).min(high),
        (3.0 * sigma).max(low).min(high),
    )
}

fn integrand(case: Case, point: [f64; 2], offset_y: f64) -> f64 {
    let half = [case.size[0] * 0.5, case.size[1] * 0.5];
    let sigma = case.blur * 0.5;
    analytic_x(point[0], point[1] - offset_y, sigma, case.radii, half) * gaussian(offset_y, sigma)
}

fn midpoint<const N: usize>(case: Case, point: [f64; 2]) -> f64 {
    let sigma = case.blur * 0.5;
    let (start, end) = bounds(point[1], case.size[1] * 0.5, sigma);
    let step = (end - start) / N as f64;
    let mut sum = 0.0;
    for index in 0..N {
        sum += integrand(case, point, start + (index as f64 + 0.5) * step);
    }
    sum * step
}

fn gauss_legendre<const N: usize>(
    case: Case,
    point: [f64; 2],
    nodes: &[f64; N],
    weights: &[f64; N],
) -> f64 {
    let sigma = case.blur * 0.5;
    let (start, end) = bounds(point[1], case.size[1] * 0.5, sigma);
    let center = (start + end) * 0.5;
    let scale = (end - start) * 0.5;
    let mut sum = 0.0;
    for index in 0..N {
        sum += weights[index] * integrand(case, point, center + scale * nodes[index]);
    }
    sum * scale
}

#[test]
fn current_four_stratum_midpoint_is_calibrated_against_gauss_legendre_and_reference() {
    let mut midpoint_4 = ErrorStats::default();
    let mut gauss_4 = ErrorStats::default();
    let mut midpoint_8 = ErrorStats::default();
    let mut gauss_8 = ErrorStats::default();

    for case in CASES {
        let mut case_midpoint_8 = ErrorStats::default();
        let mut case_gauss_8 = ErrorStats::default();
        let sigma = case.blur * 0.5;
        let span_x = case.size[0] * 0.5 + 3.0 * sigma;
        let span_y = case.size[1] * 0.5 + 3.0 * sigma;
        for grid_y in 0..GRID_SIDE {
            let y = -span_y + 2.0 * span_y * grid_y as f64 / (GRID_SIDE - 1) as f64;
            for grid_x in 0..GRID_SIDE {
                let x = -span_x + 2.0 * span_x * grid_x as f64 / (GRID_SIDE - 1) as f64;
                let point = [x, y];
                let reference = midpoint::<REFERENCE_STRATA>(case, point);
                let mid4 = midpoint::<4>(case, point);
                let gl4 = gauss_legendre(case, point, &GL4_NODES, &GL4_WEIGHTS);
                let mid8 = midpoint::<8>(case, point);
                let gl8 = gauss_legendre(case, point, &GL8_NODES, &GL8_WEIGHTS);
                midpoint_4.record(mid4, reference);
                gauss_4.record(gl4, reference);
                midpoint_8.record(mid8, reference);
                gauss_8.record(gl8, reference);
                case_midpoint_8.record(mid8, reference);
                case_gauss_8.record(gl8, reference);
            }
        }
        println!(
            "{}: midpoint8 mean={:.8} max={:.8}; GL8 mean={:.8} max={:.8}",
            case.name,
            case_midpoint_8.mean(),
            case_midpoint_8.maximum,
            case_gauss_8.mean(),
            case_gauss_8.maximum,
        );
    }

    println!(
        "aggregate: midpoint4 mean={:.8} max={:.8}; GL4 mean={:.8} max={:.8}; midpoint8 mean={:.8} max={:.8}; GL8 mean={:.8} max={:.8}",
        midpoint_4.mean(),
        midpoint_4.maximum,
        gauss_4.mean(),
        gauss_4.maximum,
        midpoint_8.mean(),
        midpoint_8.maximum,
        gauss_8.mean(),
        gauss_8.maximum,
    );

    // Global calibration limits apply to all 10,086 samples, not hand-tuned
    // per-case thresholds. The current four-stratum midpoint rule remains below
    // two alpha bytes on average and four at its worst sampled pixel. GL is the
    // conventional smooth-integrand alternative and wins numerically here; the
    // calibration documents that tradeoff rather than requiring midpoint to win.
    assert!(midpoint_4.mean() < 2.0 / 255.0);
    assert!(midpoint_4.maximum < 4.0 / 255.0);
    assert!(gauss_4.mean() < midpoint_4.mean());
    assert!(gauss_4.maximum < midpoint_4.maximum);

    // Increasing either rule to eight evaluations must materially reduce its
    // aggregate and maximum error, independently of any individual case.
    assert!(midpoint_8.mean() < midpoint_4.mean() * 0.4);
    assert!(midpoint_8.maximum < midpoint_4.maximum * 0.7);
    assert!(gauss_8.mean() < gauss_4.mean() * 0.2);
    assert!(gauss_8.maximum < gauss_4.maximum * 0.2);
}
