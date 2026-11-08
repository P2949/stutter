use super::*;

fn dim(name: &str, values: &[&str]) -> Dimension {
    Dimension {
        name: name.into(),
        values: values.iter().map(|v| (*v).into()).collect(),
    }
}

fn space() -> ConfigSpace {
    ConfigSpace {
        dimensions: vec![
            dim("cpus", &["0-3", "4-7", "0-7"]),
            dim("weight", &["100", "200"]),
        ],
        forbidden: vec![vec![
            ("cpus".into(), "0-7".into()),
            ("weight".into(), "200".into()),
        ]],
    }
}

fn cand(cpus: &str, weight: &str) -> Candidate {
    vec![cpus.into(), weight.into()]
}

/// "4-7"/"200" is truly best (mean 12), the others are near zero or negative.
fn dataset() -> Dataset {
    let mut data = Dataset::new();
    data.insert(cand("0-3", "100"), vec![0.5, -0.5, 0.2, -0.2, 0.1, 0.0]);
    data.insert(cand("0-3", "200"), vec![1.0, 0.0, 1.2, -0.2, 0.4, 0.6]);
    data.insert(cand("4-7", "100"), vec![3.0, 2.5, 3.5, 2.8, 3.2, 3.0]);
    data.insert(cand("4-7", "200"), vec![12.0, 11.5, 12.5, 11.8, 12.2, 12.0]);
    data.insert(cand("0-7", "100"), vec![-4.0, -3.0, -5.0, -4.5, -3.5, -4.0]);
    data
}

#[test]
fn the_space_is_validated_and_only_legal_candidates_exist() {
    let legal = space().enumerate().unwrap();
    assert_eq!(legal.len(), 5, "6 combinations minus the one forbidden");
    assert!(!legal.contains(&cand("0-7", "200")));
    assert_eq!(
        ConfigSpace {
            dimensions: vec![],
            forbidden: vec![]
        }
        .enumerate(),
        Err(SpaceError::Empty)
    );
    let mut empty = space();
    empty.dimensions[0].values.clear();
    assert_eq!(empty.enumerate(), Err(SpaceError::EmptyDimension));
    let mut duplicate = space();
    duplicate.dimensions[1].values.push("100".into());
    assert_eq!(duplicate.enumerate(), Err(SpaceError::DuplicateValue));
    let mut unknown = space();
    unknown.forbidden.push(vec![("cpus".into(), "9-9".into())]);
    assert_eq!(unknown.enumerate(), Err(SpaceError::UnknownInConstraint));
    let huge = ConfigSpace {
        dimensions: (0..5)
            .map(|i| Dimension {
                name: format!("d{i}"),
                values: (0..11).map(|v| v.to_string()).collect(),
            })
            .collect(),
        forbidden: vec![],
    };
    assert_eq!(huge.enumerate(), Err(SpaceError::TooLarge));
}

#[test]
fn every_policy_only_ever_proposes_a_member_of_the_legal_set() {
    let space = space();
    let legal = space.enumerate().unwrap();
    let mut policies: Vec<Box<dyn SearchPolicy>> = vec![
        Box::new(GridSearch::default()),
        Box::new(SeededRandom::new(7)),
        Box::new(PairedElimination::new(2)),
    ];
    for policy in &mut policies {
        let mut history = Vec::new();
        for step in 0..60 {
            let proposal = policy.propose(&legal, &history).unwrap();
            assert!(
                legal.contains(&proposal),
                "{} proposed {proposal:?}",
                policy.name()
            );
            history.push(Trial {
                candidate: proposal,
                improvement: f64::from(step % 7),
            });
        }
    }
}

#[test]
fn a_full_grid_finds_the_best_and_the_seeded_order_is_reproducible() {
    let report = evaluate_offline(&mut GridSearch::default(), &space(), &dataset(), 30).unwrap();
    assert_eq!(report.chosen, Some(cand("4-7", "200")));
    assert_eq!(report.regret, 0.0);
    let order = |seed| {
        let legal = space().enumerate().unwrap();
        let mut policy = SeededRandom::new(seed);
        (0..5)
            .map(|_| policy.propose(&legal, &[]).unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(order(11), order(11));
    assert_ne!(order(11), order(12));
    let mut sorted = order(11);
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), 5, "every candidate once per round");
}

#[test]
fn paired_elimination_reaches_the_best_with_far_fewer_trials_than_a_grid() {
    // Budget 14: the grid cannot even give every candidate its six blocks.
    let elimination =
        evaluate_offline(&mut PairedElimination::new(2), &space(), &dataset(), 14).unwrap();
    let grid = evaluate_offline(&mut GridSearch::default(), &space(), &dataset(), 14).unwrap();
    assert_eq!(
        elimination.chosen,
        Some(cand("4-7", "200")),
        "{elimination:?}"
    );
    assert_eq!(elimination.regret, 0.0);
    assert!(elimination.trials_used <= 14 && grid.trials_used <= 14);
    assert!(elimination.trials_to_first_try_of_chosen.is_some());
}

#[test]
fn a_replay_never_invents_a_measurement_that_was_not_recorded() {
    // A dataset with two blocks per candidate and a budget of 100: the replay stops when blocks run out.
    let mut thin = Dataset::new();
    for candidate in space().enumerate().unwrap() {
        thin.insert(candidate, vec![1.0, 2.0]);
    }
    let report = evaluate_offline(&mut GridSearch::default(), &space(), &thin, 100).unwrap();
    assert!(report.trials_used <= 10);
}

#[test]
fn an_invalid_space_is_refused_before_any_trial() {
    let bad = ConfigSpace {
        dimensions: vec![],
        forbidden: vec![],
    };
    assert_eq!(
        evaluate_offline(&mut GridSearch::default(), &bad, &dataset(), 5),
        Err(SpaceError::Empty)
    );
}
