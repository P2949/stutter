use super::*;
use crate::autotune::statistics::{
    bonferroni_confidence, paired_interval, student_t_cdf, t_quantile,
};

fn design(stopping: StoppingRule) -> ExperimentDesign {
    ExperimentDesign {
        schema_version: EXPERIMENT_SCHEMA_VERSION,
        objective: "GameRunnableLatency".into(),
        metric: "runnable_p99_ms".into(),
        unit: "ms".into(),
        candidate: "cpu_affinity_profile".into(),
        context_fingerprint: "ctx".into(),
        min_effect_percent: 10.0,
        equivalence_margin_percent: 3.0,
        seed: 42,
        block_ids: (0..40).map(|index| format!("block-{index}")).collect(),
        stopping,
        warmup_seconds: 5,
        washout_seconds: 5,
    }
}

fn fixed() -> ExperimentDesign {
    design(StoppingRule::FixedN {
        blocks: 8,
        confidence: 0.95,
    })
}

fn sequential() -> ExperimentDesign {
    design(StoppingRule::Sequential {
        looks: 4,
        blocks_per_look: 4,
        alpha: 0.05,
    })
}

#[test]
fn the_t_quantile_matches_published_values() {
    for (df, expected) in [(1.0, 12.706), (5.0, 2.5706), (10.0, 2.2281), (30.0, 2.0423)] {
        let value = t_quantile(0.975, df).unwrap();
        assert!((value - expected).abs() < 2e-3, "df {df}: {value}");
    }
    assert!((student_t_cdf(0.0, 7.0) - 0.5).abs() < 1e-9);
    assert!((student_t_cdf(1.3, 9.0) + student_t_cdf(-1.3, 9.0) - 1.0).abs() < 1e-9);
    assert!(t_quantile(1.0, 5.0).is_none() && t_quantile(0.5, 0.5).is_none());
}

#[test]
fn the_interval_of_a_known_sample_and_its_refusals() {
    let interval = paired_interval(&[1.0, 2.0, 3.0, 4.0, 5.0], 0.95).unwrap();
    assert!((interval.mean - 3.0).abs() < 1e-12);
    assert!((interval.low - 1.037).abs() < 5e-3 && (interval.high - 4.963).abs() < 5e-3);
    assert!(paired_interval(&[1.0], 0.95).is_none());
    assert!(paired_interval(&[1.0, f64::NAN], 0.95).is_none());
    assert_eq!(bonferroni_confidence(0.05, 5), Some(0.99));
    assert!(bonferroni_confidence(0.05, 0).is_none());
}

#[test]
fn a_design_is_validated_before_any_measurement() {
    assert_eq!(fixed().validate(), Ok(()));
    assert_eq!(sequential().validate(), Ok(()));
    let mut vague = fixed();
    vague.min_effect_percent = 2.0;
    assert_eq!(vague.validate(), Err(DesignError::EffectNotAboveMargin));
    let mut single_look = design(StoppingRule::Sequential {
        looks: 1,
        blocks_per_look: 4,
        alpha: 0.05,
    });
    assert_eq!(single_look.validate(), Err(DesignError::BadStoppingRule));
    single_look.stopping = StoppingRule::FixedN {
        blocks: 1,
        confidence: 0.95,
    };
    assert_eq!(single_look.validate(), Err(DesignError::TooFewBlocks));
    let mut duplicate = fixed();
    duplicate.block_ids[1] = "block-0".into();
    assert_eq!(duplicate.validate(), Err(DesignError::DuplicateBlockId));
    let mut short = fixed();
    short.block_ids.truncate(3);
    assert_eq!(
        short.validate(),
        Err(DesignError::BlockIdsDoNotCoverThePlan)
    );
    assert_ne!(
        fixed().digest(),
        design(StoppingRule::FixedN {
            blocks: 9,
            confidence: 0.95
        })
        .digest()
    );
}

#[test]
fn a_fixed_design_is_analysed_once_at_its_planned_size_and_never_early() {
    let design = fixed();
    let gains = [14.0, 15.0, 13.0, 16.0, 14.5, 15.5, 13.5, 14.2];
    assert_eq!(
        design.analyse(&gains[..5], 1, false),
        Err(AnalysisRefused::EarlyLookAtFixedN)
    );
    assert_eq!(
        design.analyse(&gains, 2, false),
        Err(AnalysisRefused::EarlyLookAtFixedN)
    );
    let analysis = design.analyse(&gains, 1, false).unwrap();
    assert_eq!(analysis.decision, Decision::Beneficial);
    // The same blocks from a drifted scenario are invalid, not a result.
    assert_eq!(
        design.analyse(&gains, 1, true).unwrap().decision,
        Decision::InvalidExperiment
    );
}

#[test]
fn outcomes_are_distinct_beneficial_equivalent_harmful_uncertain() {
    let design = fixed();
    let around =
        |center: f64| -> Vec<f64> { (0..8).map(|i| center + (i as f64 - 3.5) * 0.2).collect() };
    assert_eq!(
        design.analyse(&around(15.0), 1, false).unwrap().decision,
        Decision::Beneficial
    );
    assert_eq!(
        design.analyse(&around(0.0), 1, false).unwrap().decision,
        Decision::Equivalent
    );
    assert_eq!(
        design.analyse(&around(-12.0), 1, false).unwrap().decision,
        Decision::Harmful
    );
    let noisy = [30.0, -10.0, 25.0, -5.0, 28.0, -8.0, 22.0, 1.0];
    assert_eq!(
        design.analyse(&noisy, 1, false).unwrap().decision,
        Decision::Uncertain
    );
}

#[test]
fn a_sequential_design_pays_for_every_look_and_refuses_extra_ones() {
    let design = sequential();
    let gains: Vec<f64> = (0..16).map(|i| 14.0 + f64::from(i % 4) * 0.5).collect();
    let first = design.analyse(&gains[..4], 1, false).unwrap();
    assert!(
        (first.confidence - 0.9875).abs() < 1e-12,
        "per-look confidence is 1 - alpha/looks"
    );
    assert_eq!(first.blocks, 4);
    assert_eq!(
        design.analyse(&gains, 5, false),
        Err(AnalysisRefused::LooksExhausted)
    );
    assert_eq!(
        design.analyse(&gains[..3], 1, false),
        Err(AnalysisRefused::TooFewBlocks)
    );
    let last = design.analyse(&gains, 4, false).unwrap();
    assert_eq!(last.blocks, 16);
    assert_eq!(last.decision, Decision::Beneficial);
}

/// Deterministic standard-normal draws (xorshift64* and Box-Muller).
struct Normal(u64);

impl Normal {
    fn uniform(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }

    fn draw(&mut self) -> f64 {
        let (a, b) = (self.uniform().max(1e-12), self.uniform());
        (-2.0 * a.ln()).sqrt() * (2.0 * std::f64::consts::PI * b).cos()
    }
}

#[test]
fn repeated_unadjusted_peeking_manufactures_significance_and_the_adjusted_design_does_not() {
    // No real effect at all: every block improvement is noise around zero.
    let mut rng = Normal(0x9E37_79B9_7F4A_7C15);
    let (trials, looks, per_look) = (4000, 5, 4);
    let (mut peeking, mut adjusted) = (0, 0);
    let adjusted_confidence = bonferroni_confidence(0.05, looks).unwrap();
    for _ in 0..trials {
        let blocks: Vec<f64> = (0..looks * per_look).map(|_| rng.draw() * 8.0).collect();
        let mut peeked = false;
        let mut paid = false;
        for look in 1..=looks as usize {
            let seen = &blocks[..look * per_look as usize];
            // A naive analyst stops the first time the unadjusted 95% interval excludes zero.
            let naive = paired_interval(seen, 0.95).unwrap();
            peeked |= naive.low > 0.0 || naive.high < 0.0;
            let careful = paired_interval(seen, adjusted_confidence).unwrap();
            paid |= careful.low > 0.0 || careful.high < 0.0;
        }
        peeking += usize::from(peeked);
        adjusted += usize::from(paid);
    }
    let (peeking_rate, adjusted_rate) = (
        peeking as f64 / trials as f64,
        adjusted as f64 / trials as f64,
    );
    assert!(
        peeking_rate > 0.10,
        "unadjusted peeking should well exceed 5 percent: {peeking_rate}"
    );
    assert!(
        adjusted_rate < 0.06,
        "the adjusted design must hold its 5 percent budget: {adjusted_rate}"
    );
}

#[test]
fn evidence_must_name_the_exact_design_and_never_exceed_its_looks() {
    let design = sequential();
    let gains: Vec<f64> = (0..16).map(|i| 14.0 + f64::from(i % 4) * 0.5).collect();
    let analysis = design.analyse(&gains, 4, false).unwrap();
    let evidence = ExperimentEvidence {
        design_digest: design.digest(),
        analyses: vec![analysis.clone()],
        decision: analysis.decision,
        overhead_note: "none measured".into(),
    };
    assert!(evidence.consistent_with(&design));
    let mut other = design.clone();
    other.min_effect_percent = 9.0;
    assert!(
        !evidence.consistent_with(&other),
        "a changed threshold is a different design"
    );
    let mut too_many = evidence.clone();
    too_many.analyses = vec![analysis; 5];
    assert!(!too_many.consistent_with(&design));
    let mut wrong_decision = evidence;
    wrong_decision.decision = Decision::Harmful;
    assert!(!wrong_decision.consistent_with(&design));
}
