//! Small, auditable statistics for paired experiment blocks (package W10).
//!
//! The only tools are a Student-t quantile and the interval of a mean of independent per-block
//! improvements. Frames or requests inside one run are correlated, so the unit of analysis is the
//! independent block, never the frame. There is no learning, no p-value hunting and no hidden
//! state: the functions are pure and checked against published t quantiles.
use std::f64::consts::PI;

/// `ln Γ(x)` by the Lanczos approximation (g = 7, n = 9).
fn ln_gamma(x: f64) -> f64 {
    const COEFFICIENTS: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    if x < 0.5 {
        return (PI / (PI * x).sin()).ln() - ln_gamma(1.0 - x);
    }
    let x = x - 1.0;
    let mut sum = COEFFICIENTS[0];
    for (index, coefficient) in COEFFICIENTS.iter().enumerate().skip(1) {
        sum += coefficient / (x + index as f64);
    }
    let t = x + 7.5;
    0.5 * (2.0 * PI).ln() + (x + 0.5) * t.ln() - t + sum.ln()
}

/// Continued fraction for the regularized incomplete beta (modified Lentz).
fn beta_continued_fraction(a: f64, b: f64, x: f64) -> f64 {
    const TINY: f64 = 1e-300;
    let (qab, qap, qam) = (a + b, a + 1.0, a - 1.0);
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < TINY {
        d = TINY;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..=300 {
        let m = f64::from(m);
        let even = m * (b - m) * x / ((qam + 2.0 * m) * (a + 2.0 * m));
        d = 1.0 + even * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + even / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        h *= d * c;
        let odd = -(a + m) * (qab + m) * x / ((a + 2.0 * m) * (qap + 2.0 * m));
        d = 1.0 + odd * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + odd / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        let delta = d * c;
        h *= delta;
        if (delta - 1.0).abs() < 1e-14 {
            break;
        }
    }
    h
}

/// Regularized incomplete beta `I_x(a, b)`.
fn regularized_beta(a: f64, b: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let front =
        (ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b) + a * x.ln() + b * (1.0 - x).ln()).exp();
    if x < (a + 1.0) / (a + b + 2.0) {
        front * beta_continued_fraction(a, b, x) / a
    } else {
        1.0 - front * beta_continued_fraction(b, a, 1.0 - x) / b
    }
}

/// CDF of Student's t with `df` degrees of freedom.
pub fn student_t_cdf(t: f64, df: f64) -> f64 {
    let tail = 0.5 * regularized_beta(df / 2.0, 0.5, df / (df + t * t));
    if t >= 0.0 { 1.0 - tail } else { tail }
}

/// The `p` quantile of Student's t, `0 < p < 1`, by bisection on the CDF.
pub fn t_quantile(p: f64, df: f64) -> Option<f64> {
    if !(p > 0.0 && p < 1.0) || df < 1.0 {
        return None;
    }
    let (mut low, mut high) = (-1e4, 1e4);
    for _ in 0..200 {
        let mid = 0.5 * (low + high);
        if student_t_cdf(mid, df) < p {
            low = mid;
        } else {
            high = mid;
        }
    }
    Some(0.5 * (low + high))
}

/// A two-sided interval for the mean of independent block improvements.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Interval {
    pub mean: f64,
    pub low: f64,
    pub high: f64,
    pub blocks: usize,
    pub confidence: f64,
}

/// The interval of the mean at `confidence` (for example 0.95). `None` for fewer than two blocks,
/// a non-finite value or a confidence outside (0, 1): nothing is invented from too little data.
pub fn paired_interval(improvements: &[f64], confidence: f64) -> Option<Interval> {
    let n = improvements.len();
    if n < 2 || improvements.iter().any(|value| !value.is_finite()) {
        return None;
    }
    let mean = improvements.iter().sum::<f64>() / n as f64;
    let variance = improvements.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n as f64 - 1.0);
    let critical = t_quantile(1.0 - (1.0 - confidence) / 2.0, (n - 1) as f64)?;
    let half = critical * (variance / n as f64).sqrt();
    Some(Interval {
        mean,
        low: mean - half,
        high: mean + half,
        blocks: n,
        confidence,
    })
}

/// The per-look confidence of a Bonferroni-adjusted sequential design: the total error budget
/// `alpha` is divided over `looks`, so inspecting the data at every look never inflates it.
pub fn bonferroni_confidence(alpha: f64, looks: u32) -> Option<f64> {
    (alpha > 0.0 && alpha < 1.0 && looks >= 1).then(|| 1.0 - alpha / f64::from(looks))
}
