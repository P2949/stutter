use super::{super::*, *};

fn space() -> ConfigSpace {
    ConfigSpace {
        dimensions: vec![
            Dimension {
                name: "cpus".into(),
                values: ["0-3", "4-7", "0-7"].map(Into::into).to_vec(),
            },
            Dimension {
                name: "weight".into(),
                values: ["100", "200"].map(Into::into).to_vec(),
            },
        ],
        forbidden: vec![vec![
            ("cpus".into(), "0-7".into()),
            ("weight".into(), "200".into()),
        ]],
    }
}

fn data(rows: &[(&str, &str, &[f64])]) -> Dataset {
    rows.iter()
        .map(|(c, w, blocks)| (vec![(*c).to_owned(), (*w).to_owned()], blocks.to_vec()))
        .collect()
}

fn members() -> Vec<Box<dyn SearchPolicy>> {
    vec![
        Box::new(GridSearch::default()),
        Box::new(SeededRandom::new(7)),
        Box::new(PairedElimination::new(2)),
    ]
}

fn history() -> Dataset {
    data(&[
        ("0-3", "100", &[0.1, 0.0, 0.2, 0.1, 0.0, 0.1, 0.2, 0.0]),
        ("0-3", "200", &[1.0, 1.2, 0.8, 1.1, 1.0, 0.9, 1.2, 1.0]),
        ("4-7", "100", &[3.0, 2.8, 3.2, 3.1, 3.0, 2.9, 3.0, 3.2]),
        (
            "4-7",
            "200",
            &[12.0, 11.8, 12.2, 12.1, 12.0, 11.9, 12.0, 12.2],
        ),
        (
            "0-7",
            "100",
            &[-4.0, -3.5, -4.5, -4.0, -4.2, -3.8, -4.0, -4.1],
        ),
    ])
}

#[test]
fn the_ensemble_only_ever_proposes_legal_candidates_tries_every_member_and_is_reproducible() {
    let legal = space().enumerate().unwrap();
    let run = || {
        let mut ensemble = Ensemble::new(members());
        let report = evaluate_offline(&mut ensemble, &space(), &history(), 24).unwrap();
        (report, ensemble.pulls().to_vec())
    };
    let (first, pulls) = run();
    assert!(pulls.iter().all(|count| *count > 0), "{pulls:?}");
    assert_eq!(pulls.iter().sum::<usize>(), first.trials_used);
    assert!(legal.contains(first.chosen.as_ref().unwrap()));
    assert_eq!(
        first.regret, 0.0,
        "the true best is found within the budget"
    );
    assert_eq!(run().0, first);
}

#[test]
fn the_ensemble_never_proposes_from_an_empty_space() {
    let mut ensemble = Ensemble::new(members());
    assert_eq!(ensemble.propose(&[], &[]), None);
    let mut empty = Ensemble::new(Vec::new());
    assert_eq!(empty.propose(&space().enumerate().unwrap(), &[]), None);
}

#[test]
fn a_holdout_ranking_reports_agreement_regret_and_refuses_too_few_candidates() {
    let train = history();
    let same = held_out_ranking(&train, &history()).unwrap();
    assert!(same.top1_agrees);
    assert_eq!(same.holdout_regret, 0.0);
    assert!((same.spearman - 1.0).abs() < 1e-12);

    let inverted = data(&[
        ("0-3", "100", &[5.0]),
        ("0-3", "200", &[4.0]),
        ("4-7", "100", &[3.0]),
        ("4-7", "200", &[-9.0]),
        ("0-7", "100", &[2.0]),
    ]);
    let report = held_out_ranking(&train, &inverted).unwrap();
    assert!(!report.top1_agrees);
    assert!(report.holdout_regret < 0.0);
    assert!(report.spearman < 0.0);

    let few = data(&[("0-3", "100", &[1.0]), ("4-7", "100", &[2.0])]);
    assert_eq!(held_out_ranking(&train, &few), None);
}
