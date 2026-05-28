use super::assertions::*;
use crate::diagnosis::StutterCause;

#[test]
fn validation_corpus_cpu_pressure() {
    assert_fixture_from_metadata("cpu_pressure");
}

#[test]
fn validation_corpus_block_io_stall() {
    assert_fixture_from_metadata("block_io_stall");
}

#[test]
fn validation_corpus_irq_heavy() {
    assert_fixture_from_metadata("irq_heavy");
}

#[test]
fn validation_corpus_gpu_bound_clean_cpu_has_gpu_candidate() {
    let analysis = assert_fixture_from_metadata("gpu_bound_clean_cpu");

    assert_candidate_contains(&analysis, StutterCause::GpuBoundCandidate, &["GPU busy"]);
}

#[test]
fn validation_corpus_game_thread_scheduler_delay() {
    assert_fixture_from_metadata("game_thread_scheduler_delay");
}

#[test]
fn validation_corpus_compositor_scheduler_delay() {
    assert_fixture_from_metadata("compositor_scheduler_delay");
}

#[test]
fn validation_corpus_foreground_window() {
    let analysis = assert_fixture_from_metadata("foreground_window");

    assert_eq!(
        analysis.foreground_summary.final_pid,
        Some(stutter_core::ids::Pid::new(5701))
    );
    assert_eq!(
        analysis.foreground_summary.final_app_id.as_deref(),
        Some("steam_app_sanitized")
    );
    assert_eq!(
        analysis.foreground_summary.final_class.as_deref(),
        Some("steam_app_sanitized")
    );
    assert!(
        analysis.foreground_summary.final_title.is_none(),
        "foreground title must stay redacted"
    );
}

#[test]
fn validation_corpus_community_rules_classification() {
    let analysis = assert_fixture_from_metadata("community_rules_classification");

    let task = analysis
        .session
        .tasks
        .iter()
        .find(|task| task.comm == "community-game")
        .expect("missing community-classified task");

    assert_eq!(task.class, crate::process_tree::TaskClass::Game);
    assert_eq!(task.process_comm.as_str(), "community-game");
}
