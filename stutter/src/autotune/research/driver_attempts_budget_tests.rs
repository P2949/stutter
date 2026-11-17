use super::*;

fn read_spent(
    root: &tempfile::TempDir,
    study: &ChildStudy,
) -> crate::autotune::research::campaign_budget::Spent {
    let path = root
        .path()
        .join("studies")
        .join(&study.study_sha256[..16])
        .join("budget.json");
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
fn a_second_child_starts_from_what_the_first_already_spent_of_the_campaign() {
    let (root, mut runner, first, parent) = attempts(Host::finishing());
    let second = plan(&parent).unwrap().remove(1);
    run_to_end(&mut runner, &first);
    let spent = read_spent(&root, &first);
    assert!(
        spent.active_seconds > 0,
        "the first child's time was charged"
    );
    // The second child's driver does not start from zero: the campaign's budget is shared.
    assert_eq!(runner.poll(&second, 100).unwrap(), None);
    let (_, driver) = runner.running.as_ref().unwrap();
    assert_eq!(driver.active_seconds(), spent.active_seconds);
}

#[test]
fn a_child_that_would_start_over_an_exhausted_campaign_budget_is_stopped_at_once() {
    let (root, mut runner, first, parent) = attempts(Host::finishing());
    let second = plan(&parent).unwrap().remove(1);
    run_to_end(&mut runner, &first);
    // Pretend the first child used the whole authorization (its record is rewritten before the
    // second begins, as a campaign that had spent that much would have recorded).
    let path = root
        .path()
        .join("studies")
        .join(&first.study_sha256[..16])
        .join("budget.json");
    let mut spent = read_spent(&root, &first);
    spent.active_seconds = u64::from(parent.max_active_seconds);
    std::fs::write(&path, serde_json::to_vec(&spent).unwrap()).unwrap();
    let outcome = run_to_end(&mut runner, &second);
    assert_eq!(
        outcome.attempts_used, 0,
        "no attempt may start over an exhausted active budget"
    );
}
