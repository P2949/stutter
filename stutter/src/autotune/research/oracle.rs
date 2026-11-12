//! Scoring of Stutter's live diagnosis against pre-registered, independently generated ground
//! truth. Ground truth is fixed before Stutter's output is read; high FPS is never evidence of
//! runnable-delay relief; and any action on a non-CPU-contention case counts as unsafe.

use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Cause {
    RunnableDelay,
    GpuBound,
    ColdShaderCache,
    IoPressure,
    OwnerConflict,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroundTruth {
    pub case_id: String,
    /// `None` is a matched negative control: no injected cause.
    pub injected: Option<Cause>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnosis {
    pub case_id: String,
    pub reported: BTreeSet<Cause>,
    pub abstained: bool,
    pub action_proposed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaseOutcome {
    Included,
    Missed,
    CorrectAbstention,
    FalsePositive(BTreeSet<Cause>),
    /// Reported a cause (or acted) where an action is never supported.
    UnsafeAction,
}

pub fn score_case(truth: &GroundTruth, diagnosis: &Diagnosis) -> anyhow::Result<CaseOutcome> {
    anyhow::ensure!(
        truth.case_id == diagnosis.case_id,
        "diagnosis is for a different case"
    );
    anyhow::ensure!(
        !diagnosis.abstained || (diagnosis.reported.is_empty() && !diagnosis.action_proposed),
        "abstention contradicts reported causes or an action"
    );
    // Only runnable delay can ever justify a CPU placement action.
    if diagnosis.action_proposed && truth.injected != Some(Cause::RunnableDelay) {
        return Ok(CaseOutcome::UnsafeAction);
    }
    Ok(match truth.injected {
        Some(cause) if diagnosis.reported.contains(&cause) => {
            let extra: BTreeSet<_> = diagnosis
                .reported
                .iter()
                .copied()
                .filter(|c| *c != cause)
                .collect();
            if extra.is_empty() {
                CaseOutcome::Included
            } else {
                CaseOutcome::FalsePositive(extra)
            }
        }
        Some(_) if !diagnosis.reported.is_empty() => {
            CaseOutcome::FalsePositive(diagnosis.reported.clone())
        }
        Some(_) => CaseOutcome::Missed,
        None if diagnosis.reported.is_empty() && diagnosis.abstained => {
            CaseOutcome::CorrectAbstention
        }
        None => CaseOutcome::FalsePositive(diagnosis.reported.clone()),
    })
}

#[derive(Debug, PartialEq)]
pub struct Summary {
    pub positives: usize,
    pub included: usize,
    pub negatives: usize,
    pub correct_abstentions: usize,
    pub false_positive_cases: usize,
    pub unsafe_actions: usize,
}

pub fn summarize(truths: &[GroundTruth], diagnoses: &[Diagnosis]) -> anyhow::Result<Summary> {
    anyhow::ensure!(
        truths.len() == diagnoses.len(),
        "every case needs exactly one diagnosis"
    );
    let mut ids = BTreeSet::new();
    anyhow::ensure!(
        truths
            .iter()
            .all(|truth| !truth.case_id.is_empty() && ids.insert(&truth.case_id)),
        "case identities must be nonempty and unique"
    );
    let mut summary = Summary {
        positives: 0,
        included: 0,
        negatives: 0,
        correct_abstentions: 0,
        false_positive_cases: 0,
        unsafe_actions: 0,
    };
    for (truth, diagnosis) in truths.iter().zip(diagnoses) {
        match truth.injected {
            Some(_) => summary.positives += 1,
            None => summary.negatives += 1,
        }
        match score_case(truth, diagnosis)? {
            CaseOutcome::Included => summary.included += 1,
            CaseOutcome::Missed => {}
            CaseOutcome::CorrectAbstention => summary.correct_abstentions += 1,
            CaseOutcome::FalsePositive(_) => summary.false_positive_cases += 1,
            CaseOutcome::UnsafeAction => summary.unsafe_actions += 1,
        }
    }
    Ok(summary)
}

/// The P6 gate: two causal positives included, a GPU-bound negative left without action, and no
/// unsafe action anywhere.
pub fn gate_passes(truths: &[GroundTruth], diagnoses: &[Diagnosis]) -> anyhow::Result<bool> {
    let summary = summarize(truths, diagnoses)?;
    let gpu_negative_clean = truths.iter().zip(diagnoses).any(|(t, d)| {
        t.injected == Some(Cause::GpuBound)
            && !d.action_proposed
            && (matches!(score_case(t, d), Ok(CaseOutcome::Included))
                || (d.abstained && d.reported.is_empty()))
    });
    let causal_hits: BTreeSet<_> = truths
        .iter()
        .zip(diagnoses)
        .filter_map(|(t, d)| match t.injected {
            Some(cause @ (Cause::RunnableDelay | Cause::IoPressure))
                if matches!(score_case(t, d), Ok(CaseOutcome::Included)) =>
            {
                Some(cause)
            }
            _ => None,
        })
        .collect();
    Ok(causal_hits.len() == 2
        && gpu_negative_clean
        && summary.negatives > 0
        && summary.correct_abstentions == summary.negatives
        && summary.false_positive_cases == 0
        && summary.unsafe_actions == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn truth(id: &str, injected: Option<Cause>) -> GroundTruth {
        GroundTruth {
            case_id: id.into(),
            injected,
        }
    }
    fn dx(id: &str, reported: &[Cause], abstained: bool, action: bool) -> Diagnosis {
        Diagnosis {
            case_id: id.into(),
            reported: reported.iter().copied().collect(),
            abstained,
            action_proposed: action,
        }
    }

    #[test]
    fn scores_inclusion_false_positives_abstention_and_unsafe_actions() {
        use Cause::*;
        assert_eq!(
            score_case(
                &truth("a", Some(RunnableDelay)),
                &dx("a", &[RunnableDelay], false, true)
            )
            .unwrap(),
            CaseOutcome::Included
        );
        assert!(matches!(
            score_case(
                &truth("a", Some(RunnableDelay)),
                &dx("a", &[GpuBound], false, false)
            )
            .unwrap(),
            CaseOutcome::FalsePositive(_)
        ));
        assert!(matches!(
            score_case(
                &truth("a", Some(RunnableDelay)),
                &dx("a", &[RunnableDelay, IoPressure], false, false)
            )
            .unwrap(),
            CaseOutcome::FalsePositive(_)
        ));
        assert_eq!(
            score_case(&truth("n", None), &dx("n", &[], true, false)).unwrap(),
            CaseOutcome::CorrectAbstention
        );
        assert!(matches!(
            score_case(&truth("n", None), &dx("n", &[RunnableDelay], false, false)).unwrap(),
            CaseOutcome::FalsePositive(_)
        ));
        // An affinity-style action while GPU-bound is unsafe even if the cause was reported right.
        assert_eq!(
            score_case(
                &truth("g", Some(GpuBound)),
                &dx("g", &[GpuBound], false, true)
            )
            .unwrap(),
            CaseOutcome::UnsafeAction
        );
        // A cold-cache transient is not an owner conflict, and vice versa.
        assert!(matches!(
            score_case(
                &truth("c", Some(ColdShaderCache)),
                &dx("c", &[OwnerConflict], false, false)
            )
            .unwrap(),
            CaseOutcome::FalsePositive(_)
        ));
        assert!(score_case(&truth("a", None), &dx("b", &[], true, false)).is_err());
    }

    #[test]
    fn the_gate_needs_two_causal_hits_a_clean_gpu_negative_and_no_unsafe_action() {
        use Cause::*;
        let truths = vec![
            truth("c1", Some(RunnableDelay)),
            truth("c2", Some(IoPressure)),
            truth("g", Some(GpuBound)),
            truth("n", None),
        ];
        let good = vec![
            dx("c1", &[RunnableDelay], false, true),
            dx("c2", &[IoPressure], false, false),
            dx("g", &[GpuBound], false, false),
            dx("n", &[], true, false),
        ];
        assert!(gate_passes(&truths, &good).unwrap());
        let mut bad = good.clone();
        bad[2].action_proposed = true;
        assert!(!gate_passes(&truths, &bad).unwrap());
        let mut missed = good.clone();
        missed[1] = dx("c2", &[], true, false);
        assert!(!gate_passes(&truths, &missed).unwrap());
        let mut false_positive = good.clone();
        false_positive[3] = dx("n", &[RunnableDelay], false, false);
        assert!(!gate_passes(&truths, &false_positive).unwrap());
        let mut wrong_gpu = good.clone();
        wrong_gpu[2] = dx("g", &[RunnableDelay], false, false);
        assert!(!gate_passes(&truths, &wrong_gpu).unwrap());
        assert!(!gate_passes(&truths[..3], &good[..3]).unwrap());
        assert!(score_case(&truths[0], &dx("c1", &[RunnableDelay], true, false)).is_err());
        let mut duplicate = truths.clone();
        duplicate[1].case_id = duplicate[0].case_id.clone();
        assert!(gate_passes(&duplicate, &good).is_err());
        assert!(gate_passes(&truths, &good[..3]).is_err());
    }
}
