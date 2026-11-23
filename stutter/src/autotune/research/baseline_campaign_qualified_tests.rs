use super::*;

/// The controller applies the action in every attempt the schedule treats On, each on its own game process, and the
/// collectors seal the paired runnable-latency measurement before the study ends: the whole production-composed
/// campaign qualifies the action and independent verification replays the receipts, the measurement and the budget.
#[test]
fn an_applied_action_with_its_sealed_measurement_qualifies_through_the_whole_campaign_and_verifies()
{
    let mut run_of = campaign_that_acts(1_500_000);
    run(&mut run_of);
    let dispositions = run_of.driver.dispositions();
    let action = dispositions
        .iter()
        .find(|d| matches!(d.stage, Stage::Action(_)))
        .expect("the action study is planned");
    assert_eq!(
        (action.result, action.reason.as_str()),
        (Some(StudyResult::Qualified), ""),
        "{dispositions:?}"
    );
    let verified = verify_all(&run_of);
    assert!(
        verified.children.iter().all(|c| c.problem.is_none()),
        "{:?}",
        verified.children
    );
    assert!(verified.all_verified(), "{:?}", verified.budget_problem);
    assert!(verified.actions_restored(&dispositions));
    // The independently reconstructed runnable-latency result reaches the verification the bundle is built from.
    let measured: Vec<_> = verified
        .children
        .iter()
        .filter_map(|c| c.action_latency_result.as_ref())
        .collect();
    assert_eq!(measured.len(), 1, "{:?}", verified.children);
    assert_eq!(measured[0]["decision"], "keep");
    assert_eq!(measured[0]["objective"], "game_runnable_latency");
    let dir = study_with(&run_of, "action-receipts.json");
    let receipts: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("action-receipts.json")).unwrap()).unwrap();
    assert_eq!(
        receipts.as_array().unwrap().len(),
        4,
        "one per treated attempt"
    );
}

/// Altering what the verifier replays is caught: the sealed measurement removed, a receipt's restoration changed, the
/// controller's journal archive removed, and a receipt dropped (a partly applied action) each contradict the claimed qualification.
#[test]
fn a_qualified_action_campaign_is_refused_when_its_measurement_or_receipts_are_altered() {
    for tamper in 0..5 {
        let mut run_of = campaign_that_acts(1_500_000);
        run(&mut run_of);
        let dispositions = run_of.driver.dispositions();
        let dir = study_with(&run_of, "action-receipts.json");
        let receipts_path = dir.join("action-receipts.json");
        let mut receipts: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&receipts_path).unwrap()).unwrap();
        match tamper {
            0 => std::fs::remove_file(dir.join(crate::autotune::research::action_latency::FILE))
                .unwrap(),
            1 => {
                receipts[0]["restored"]["cpus_allowed"] = "0-3".into();
                rewrite(&receipts_path, &serde_json::to_vec(&receipts).unwrap());
            }
            3 => std::fs::remove_file(
                dir.join(crate::autotune::research::controller_provenance::FILE),
            )
            .unwrap(),
            4 => std::fs::remove_file(dir.join(crate::autotune::research::worker_provenance::FILE))
                .unwrap(),
            _ => {
                receipts.as_array_mut().unwrap().pop();
                rewrite(&receipts_path, &serde_json::to_vec(&receipts).unwrap());
            }
        }
        let verified = verify_all(&run_of);
        assert!(
            verified.children.iter().any(|c| c.problem.is_some())
                || !verified.actions_restored(&dispositions),
            "tamper {tamper} was not caught"
        );
    }
}

/// An action that made the runnable tail worse is a valid, applied and restored measurement: the study is a
/// qualified one, and what the production decision says of it is Revert, never a benefit.
#[test]
fn an_action_that_made_the_tail_worse_is_a_valid_study_whose_decision_is_revert() {
    use crate::autotune::research::{
        action_latency::{self, Decision},
        child_verify_rules::processes_of,
        treatment::treatments_of,
    };
    let mut run_of = campaign_that_acts(40_000_000);
    run(&mut run_of);
    let dispositions = run_of.driver.dispositions();
    let action = dispositions
        .iter()
        .find(|d| matches!(d.stage, Stage::Action(_)))
        .unwrap();
    assert_eq!(
        action.result,
        Some(StudyResult::Qualified),
        "{dispositions:?}"
    );
    let verified = verify_all(&run_of);
    assert!(verified.children.iter().all(|c| c.problem.is_none()));
    // A valid study of an action that made the tail worse carries a Revert result, not a benefit.
    let reverted: Vec<_> = verified
        .children
        .iter()
        .filter_map(|c| c.action_latency_result.as_ref())
        .collect();
    assert_eq!(reverted.len(), 1);
    assert_eq!(reverted[0]["decision"], "revert");
    let dir = study_with(&run_of, action_latency::FILE);
    let intent: ResearchIntentV1 =
        serde_json::from_slice(&std::fs::read(dir.join("intent.json")).unwrap()).unwrap();
    let evidence: std::collections::BTreeMap<
        u32,
        crate::autotune::research::report::AttemptEvidenceV1,
    > = std::fs::read_dir(dir.join("evidence"))
        .unwrap()
        .flatten()
        .map(|entry| {
            let item: crate::autotune::research::report::AttemptEvidenceV1 =
                serde_json::from_slice(&std::fs::read(entry.path()).unwrap()).unwrap();
            (item.slot_index, item)
        })
        .collect();
    let result = action_latency::verify(
        &std::fs::read(dir.join(action_latency::FILE)).unwrap(),
        &treatments_of(&intent).unwrap().unwrap(),
        &processes_of(&evidence),
    )
    .unwrap();
    assert_eq!(result.decision, Decision::Revert);
}

/// A feature through the whole production-composed campaign: the treatment reached the game process, the kernel's
/// witness shows the factor's modelled backend (the ntsync device) on every attempt treated On and on none treated Off,
/// each attempt names its own game process, and independent verification replays the study, its proof and the budget.
#[test]
fn a_feature_whose_backend_the_kernel_showed_on_every_treated_attempt_qualifies_through_the_whole_campaign()
 {
    use crate::autotune::research::ResearchFactor;
    let mut run_of = build_campaign(None, Some(ResearchFactor::NtSync));
    run(&mut run_of);
    let dispositions = run_of.driver.dispositions();
    let feature = dispositions
        .iter()
        .find(|d| matches!(d.stage, Stage::Feature(_)))
        .expect("the feature study is planned");
    assert_eq!(
        (feature.result, feature.reason.as_str()),
        (Some(StudyResult::Qualified), ""),
        "{dispositions:?}"
    );
    let verified = verify_all(&run_of);
    assert!(
        verified.children.iter().all(|c| c.problem.is_none()),
        "{:?}",
        verified
            .children
            .iter()
            .map(|c| (&c.study, &c.problem))
            .collect::<Vec<_>>()
    );
    assert!(verified.all_verified(), "{:?}", verified.budget_problem);
    // The proof the study stood on is beside it, and removing it contradicts the claimed qualification.
    let dir = study_with(&run_of, crate::autotune::research::feature_proof::FILE);
    std::fs::remove_file(dir.join(crate::autotune::research::feature_proof::FILE)).unwrap();
    let tampered = verify_all(&run_of);
    assert!(tampered.children.iter().any(|c| c.problem.is_some()));
}
