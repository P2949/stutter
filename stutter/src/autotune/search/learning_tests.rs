use super::{super::*, *};

fn space() -> ConfigSpace {
    ConfigSpace {
        dimensions: vec![Dimension {
            name: "block".into(),
            values: ["16", "32", "64", "128"].map(Into::into).to_vec(),
        }],
        forbidden: vec![],
    }
}

fn export(trials: &str) -> Vec<u8> {
    format!(r#"{{"source":"kernel_tuner","trials":[{trials}]}}"#).into_bytes()
}

#[test]
fn a_wrong_answer_removes_its_candidate_and_unverified_or_foreign_trials_are_not_used() {
    let json = export(
        r#"{"candidate":["16"],"improvement":1.0,"correct":true},
           {"candidate":["32"],"improvement":9.0,"correct":true},
           {"candidate":["32"],"improvement":9.5,"correct":false},
           {"candidate":["64"],"improvement":3.0},
           {"candidate":["999"],"improvement":50.0,"correct":true}"#,
    );
    let imported = import_measurements(&json, &space()).unwrap();
    assert_eq!(imported.dataset.len(), 1, "only 16 is verified and correct");
    assert_eq!(imported.dataset[&vec!["16".to_owned()]], vec![1.0]);
    assert_eq!(imported.incorrect_candidates, 1);
    assert_eq!(imported.unverified, 1);
    assert_eq!(imported.outside_space, 1);
}

#[test]
fn malformed_empty_or_non_finite_exports_are_errors() {
    for bad in [
        b"not json".to_vec(),
        export(""),
        export(r#"{"candidate":["16"],"improvement":1.0}"#),
        br#"{"source":" ","trials":[]}"#.to_vec(),
        br#"{"source":"x","trials":[{"candidate":["16"],"improvement":1e999,"correct":true}]}"#
            .to_vec(),
    ] {
        assert!(import_measurements(&bad, &space()).is_err());
    }
}

#[test]
fn the_bayesian_policy_stays_legal_looks_at_each_candidate_and_finds_the_best() {
    let legal = space().enumerate().unwrap();
    let mut data = Dataset::new();
    for (value, blocks) in [
        ("16", [0.1, 0.0, 0.2, 0.1]),
        ("32", [2.0, 2.2, 1.8, 2.0]),
        ("64", [7.0, 7.2, 6.8, 7.0]),
        ("128", [-1.0, -1.2, -0.8, -1.0]),
    ] {
        data.insert(vec![value.to_owned()], blocks.to_vec());
    }
    let mut policy = OptimisticBayes::new(1.0, 1.0);
    let report = evaluate_offline(&mut policy, &space(), &data, 12).unwrap();
    assert_eq!(report.chosen, Some(vec!["64".to_owned()]));
    assert_eq!(report.regret, 0.0);
    // The replay ends when the favourite runs out of recorded blocks: it never invents one.
    assert!(report.trials_used <= 12);

    let mut seen = Vec::new();
    let mut policy = OptimisticBayes::new(1.0, 1.0);
    let mut history = Vec::new();
    for _ in 0..4 {
        let candidate = policy.propose(&legal, &history).unwrap();
        assert!(legal.contains(&candidate));
        seen.push(candidate.clone());
        history.push(Trial {
            candidate,
            improvement: 1.0,
        });
    }
    seen.sort();
    seen.dedup();
    assert_eq!(
        seen.len(),
        4,
        "every candidate is tried once before repeating"
    );
    assert_eq!(OptimisticBayes::new(1.0, 1.0).propose(&[], &[]), None);
}
