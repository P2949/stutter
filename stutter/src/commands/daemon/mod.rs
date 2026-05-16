use crate::daemon::{
    DaemonAcceptanceReport, default_daemon_state_snapshot_path, run_fake_daemon_acceptance_suite,
};

pub mod config;
pub mod doctor;
pub mod explain;
pub mod helpers;
pub mod overhead;
pub mod policy;
pub mod profiles;
pub mod reset;
pub mod restore;
pub mod soak;
pub mod status;
pub mod watch;

pub fn run_config_explain_command(
    input: crate::commands::input::DaemonConfigExplainCommandInput,
) -> anyhow::Result<()> {
    config::run_config_explain_command(input)
}

pub fn run_policy_explain_command(
    input: crate::commands::input::DaemonPolicyExplainCommandInput,
) -> anyhow::Result<()> {
    policy::run_policy_explain_command(input)
}

pub fn run_privileged_worker_command(
    input: crate::commands::input::PrivilegedWorkerCommandInput,
) -> anyhow::Result<()> {
    crate::daemon::privilege::run_privileged_worker(&input.socket)
}

pub fn run_profiles_command(
    input: crate::commands::input::DaemonProfilesCommandInput,
) -> anyhow::Result<()> {
    profiles::run_profiles_command(input)
}

pub fn run_explain_command(
    input: crate::commands::input::DaemonExplainCommandInput,
) -> anyhow::Result<()> {
    explain::run_explain_command(input)
}

pub fn run_why_not_optimize_command(
    input: crate::commands::input::DaemonWhyNotOptimizeCommandInput,
) -> anyhow::Result<()> {
    explain::run_why_not_optimize_command(input)
}

pub fn run_what_changed_command(
    input: crate::commands::input::DaemonWhatChangedCommandInput,
) -> anyhow::Result<()> {
    explain::run_what_changed_command(input)
}

pub fn run_status_command(
    input: crate::commands::input::DaemonStatusCommandInput,
) -> anyhow::Result<()> {
    status::run_status_command(input)
}

pub fn run_watch_command(
    input: crate::commands::input::DaemonWatchCommandInput,
) -> anyhow::Result<()> {
    watch::run_watch_command(input)
}

pub fn run_doctor_command(
    input: crate::commands::input::DaemonDoctorCommandInput,
) -> anyhow::Result<()> {
    doctor::run_doctor_command(input)
}

pub fn run_reset_state_command(
    input: crate::commands::input::DaemonResetStateCommandInput,
) -> anyhow::Result<()> {
    reset::run_reset_state_command(input)
}

pub fn run_bench_overhead_command(
    input: crate::commands::input::DaemonBenchOverheadCommandInput,
) -> anyhow::Result<()> {
    overhead::run_bench_overhead_command(input)
}

pub fn run_soak_command(
    input: crate::commands::input::DaemonSoakCommandInput,
) -> anyhow::Result<()> {
    soak::run_soak_command(input)
}

pub fn run_acceptance_command(
    input: crate::commands::input::DaemonAcceptanceCommandInput,
) -> anyhow::Result<()> {
    let report = run_fake_daemon_acceptance_suite();

    if input.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", render_acceptance_text(&report));
    }

    if !report.passed {
        anyhow::bail!("daemon acceptance suite failed one or more steps");
    }

    Ok(())
}

pub fn run_pause_command(_: crate::commands::input::DaemonPauseCommandInput) -> anyhow::Result<()> {
    let state_path = default_daemon_state_snapshot_path();
    let mut store = helpers::daemon_state_store_for_path(&state_path)?;

    store.pause("operator requested daemon pause")?;

    println!(
        "daemon paused; state_path={} manual_restore_command=\"stutter daemon emergency-restore\"",
        state_path.display()
    );
    Ok(())
}

pub fn run_resume_command(
    _: crate::commands::input::DaemonResumeCommandInput,
) -> anyhow::Result<()> {
    let state_path = default_daemon_state_snapshot_path();
    let mut store = helpers::daemon_state_store_for_path(&state_path)?;

    store.resume("operator requested daemon resume")?;

    println!("daemon resumed; state_path={}", state_path.display());
    Ok(())
}

pub fn run_restore_command(
    input: crate::commands::input::DaemonRestoreCommandInput,
) -> anyhow::Result<()> {
    restore::run_restore_command(input)
}

pub fn render_acceptance_text(report: &DaemonAcceptanceReport) -> String {
    let mut text = String::new();

    text.push_str("Daemon acceptance\n");
    text.push_str("=================\n");
    text.push_str(&format!("suite: {}\n", report.suite));
    text.push_str(&format!("passed: {}\n", report.passed));

    for step in &report.steps {
        text.push_str(&format!(
            "step {} {}: {} - {}\n",
            step.number,
            step.code,
            if step.passed { "passed" } else { "failed" },
            step.evidence
        ));
    }

    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_acceptance_text_lists_final_boss_steps() {
        let report = crate::daemon::run_fake_daemon_acceptance_suite();

        let text = render_acceptance_text(&report);

        assert!(text.contains("Daemon acceptance"));
        assert!(text.contains("suite: fake-daemon-100-percent-acceptance"));
        assert!(text.contains("passed: true"));
        assert!(text.contains("step 1 install_service: passed"));
        assert!(text.contains("step 22 complete_audit_history: passed"));
    }
}
