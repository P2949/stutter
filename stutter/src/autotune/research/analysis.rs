//! Run-level paired analysis for Proton comparisons.
//!
//! The unit of replication is one whole native benchmark run. Frames are never independent trials.
//! The verdict is a pure function of the frozen rule and the paired differences, so an offline
//! verifier reproduces it exactly. Missing or non-finite data refuses analysis; it is never zero.

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Rule {
    /// Smallest relative effect that matters (for example 0.05).
    pub(crate) useful_effect: f64,
    /// Relative half-width inside which the arms are called equivalent.
    pub(crate) equivalence_margin: f64,
    /// Family-wise error rate before multiplicity correction.
    pub(crate) alpha: f64,
    /// Number of simultaneous comparisons in the family (Bonferroni).
    pub(crate) comparisons: usize,
    pub(crate) min_pairs: usize,
    /// True when larger metric values are better (FPS); false for frame time.
    pub(crate) higher_is_better: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    Improved,
    Regressed,
    Equivalent,
    NoUsefulEffect,
    Inconclusive,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Refused {
    InvalidRule,
    TooFewPairs,
    NonFinite,
    NonPositiveBaseline,
    UnequalPairing,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Outcome {
    pub(crate) pairs: usize,
    /// Mean of per-pair relative effects `(candidate - control) / control`, sign-adjusted so
    /// positive always means better.
    pub(crate) mean_effect: f64,
    pub(crate) ci_low: f64,
    pub(crate) ci_high: f64,
    pub(crate) verdict: Verdict,
}

fn ln_gamma(x: f64) -> f64 {
    const C: [f64; 6] = [
        76.180_091_729_471_46,
        -86.505_320_329_416_77,
        24.014_098_240_830_91,
        -1.231_739_572_450_155,
        0.001_208_650_973_866_179,
        -0.000_005_395_239_384_953,
    ];
    let mut y = x;
    let tmp = x + 5.5;
    let tmp = tmp - (x + 0.5) * tmp.ln();
    let mut ser = 1.000_000_000_190_015;
    for coefficient in C {
        y += 1.0;
        ser += coefficient / y;
    }
    -tmp + (2.506_628_274_631_000_5 * ser / x).ln()
}

fn beta_cf(a: f64, b: f64, x: f64) -> f64 {
    let tiny = 1e-300;
    let (qab, qap, qam) = (a + b, a + 1.0, a - 1.0);
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < tiny {
        d = tiny;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..300 {
        let m = f64::from(m);
        let m2 = 2.0 * m;
        let aa = m * (b - m) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < tiny {
            d = tiny;
        }
        c = 1.0 + aa / c;
        if c.abs() < tiny {
            c = tiny;
        }
        d = 1.0 / d;
        h *= d * c;
        let aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < tiny {
            d = tiny;
        }
        c = 1.0 + aa / c;
        if c.abs() < tiny {
            c = tiny;
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

fn reg_inc_beta(a: f64, b: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let front =
        (ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b) + a * x.ln() + b * (1.0 - x).ln()).exp();
    if x < (a + 1.0) / (a + b + 2.0) {
        front * beta_cf(a, b, x) / a
    } else {
        1.0 - front * beta_cf(b, a, 1.0 - x) / b
    }
}

/// Student t CDF.
fn t_cdf(t: f64, df: f64) -> f64 {
    let tail = 0.5 * reg_inc_beta(df / 2.0, 0.5, df / (df + t * t));
    if t >= 0.0 { 1.0 - tail } else { tail }
}

/// Two-sided critical value for tail probability `p` in each tail.
pub(crate) fn t_critical(p: f64, df: f64) -> f64 {
    let (mut low, mut high) = (0.0_f64, 1000.0_f64);
    for _ in 0..200 {
        let mid = 0.5 * (low + high);
        if 1.0 - t_cdf(mid, df) > p {
            low = mid;
        } else {
            high = mid;
        }
    }
    0.5 * (low + high)
}

/// Compare paired runs. `control` and `candidate` are native per-run values, index-aligned.
pub(crate) fn analyse_pairs(
    rule: &Rule,
    control: &[f64],
    candidate: &[f64],
) -> Result<Outcome, Refused> {
    if !(rule.alpha > 0.0 && rule.alpha < 1.0)
        || rule.comparisons == 0
        || !(rule.useful_effect > 0.0 && rule.useful_effect.is_finite())
        || !(rule.equivalence_margin > 0.0 && rule.equivalence_margin.is_finite())
        || rule.min_pairs < 2
    {
        return Err(Refused::InvalidRule);
    }
    if control.len() != candidate.len() {
        return Err(Refused::UnequalPairing);
    }
    if control.len() < rule.min_pairs {
        return Err(Refused::TooFewPairs);
    }
    if control.iter().chain(candidate).any(|v| !v.is_finite()) {
        return Err(Refused::NonFinite);
    }
    if control.iter().any(|v| *v <= 0.0) {
        return Err(Refused::NonPositiveBaseline);
    }
    let sign = if rule.higher_is_better { 1.0 } else { -1.0 };
    let effects: Vec<f64> = control
        .iter()
        .zip(candidate)
        .map(|(c, k)| sign * (k - c) / c)
        .collect();
    let n = effects.len() as f64;
    let mean = effects.iter().sum::<f64>() / n;
    let variance = effects.iter().map(|e| (e - mean).powi(2)).sum::<f64>() / (n - 1.0);
    let half =
        t_critical(rule.alpha / rule.comparisons as f64 / 2.0, n - 1.0) * (variance / n).sqrt();
    let (low, high) = (mean - half, mean + half);
    let verdict = if low >= rule.useful_effect {
        Verdict::Improved
    } else if high <= -rule.useful_effect {
        Verdict::Regressed
    } else if low >= -rule.equivalence_margin && high <= rule.equivalence_margin {
        Verdict::Equivalent
    } else if high < rule.useful_effect && low > -rule.useful_effect {
        Verdict::NoUsefulEffect
    } else {
        Verdict::Inconclusive
    };
    Ok(Outcome {
        pairs: effects.len(),
        mean_effect: mean,
        ci_low: low,
        ci_high: high,
        verdict,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule() -> Rule {
        Rule {
            useful_effect: 0.05,
            equivalence_margin: 0.03,
            alpha: 0.05,
            comparisons: 1,
            min_pairs: 4,
            higher_is_better: true,
        }
    }

    #[test]
    fn t_quantile_matches_tables() {
        assert!((t_critical(0.025, 5.0) - 2.5706).abs() < 1e-3);
        assert!((t_critical(0.025, 30.0) - 2.0423).abs() < 1e-3);
    }

    #[test]
    fn clear_gain_regression_and_equivalence() {
        let control = [100.0, 101.0, 99.0, 100.0, 100.5, 99.5];
        let better: Vec<f64> = control.iter().map(|v| v * 1.15).collect();
        assert_eq!(
            analyse_pairs(&rule(), &control, &better).unwrap().verdict,
            Verdict::Improved
        );
        let worse: Vec<f64> = control.iter().map(|v| v * 0.85).collect();
        assert_eq!(
            analyse_pairs(&rule(), &control, &worse).unwrap().verdict,
            Verdict::Regressed
        );
        let same: Vec<f64> = control
            .iter()
            .enumerate()
            .map(|(i, v)| v * (1.0 + 0.002 * (i as f64 - 2.5)))
            .collect();
        assert_eq!(
            analyse_pairs(&rule(), &control, &same).unwrap().verdict,
            Verdict::Equivalent
        );
    }

    #[test]
    fn noisy_small_effect_is_inconclusive_not_equivalent() {
        let control = [100.0, 100.0, 100.0, 100.0];
        let noisy = [112.0, 90.0, 108.0, 96.0];
        assert_eq!(
            analyse_pairs(&rule(), &control, &noisy).unwrap().verdict,
            Verdict::Inconclusive
        );
    }

    #[test]
    fn multiplicity_widens_the_interval() {
        let control = [100.0, 100.0, 100.0, 100.0, 100.0];
        let cand = [106.0, 107.0, 105.0, 108.0, 106.5];
        let one = analyse_pairs(&rule(), &control, &cand).unwrap();
        let mut family = rule();
        family.comparisons = 15;
        let many = analyse_pairs(&family, &control, &cand).unwrap();
        assert!(many.ci_high - many.ci_low > one.ci_high - one.ci_low);
    }

    #[test]
    fn lower_is_better_flips_sign() {
        let mut frame_time = rule();
        frame_time.higher_is_better = false;
        let control = [10.0, 10.1, 9.9, 10.0];
        let faster: Vec<f64> = control.iter().map(|v| v * 0.8).collect();
        assert_eq!(
            analyse_pairs(&frame_time, &control, &faster)
                .unwrap()
                .verdict,
            Verdict::Improved
        );
    }

    #[test]
    fn bad_data_refuses_instead_of_defaulting() {
        let r = rule();
        assert_eq!(
            analyse_pairs(&r, &[1.0; 4], &[1.0; 3]),
            Err(Refused::UnequalPairing)
        );
        assert_eq!(
            analyse_pairs(&r, &[1.0; 3], &[1.0; 3]),
            Err(Refused::TooFewPairs)
        );
        assert_eq!(
            analyse_pairs(&r, &[1.0; 4], &[1.0, f64::NAN, 1.0, 1.0]),
            Err(Refused::NonFinite)
        );
        assert_eq!(
            analyse_pairs(&r, &[0.0; 4], &[1.0; 4]),
            Err(Refused::NonPositiveBaseline)
        );
        let mut bad = rule();
        bad.alpha = 2.0;
        assert_eq!(
            analyse_pairs(&bad, &[1.0; 4], &[1.0; 4]),
            Err(Refused::InvalidRule)
        );
    }
}
