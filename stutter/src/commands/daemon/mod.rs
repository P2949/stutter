pub mod acceptance;
pub mod config;
pub mod doctor;
pub mod explain;
pub mod helpers;
pub mod lifecycle;
pub mod overhead;
pub mod policy;
pub mod privileged_worker;
pub mod profiles;
pub mod reset;
pub mod restore;
pub mod resync;
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

pub fn run_policy_lint_command(
    input: crate::commands::input::DaemonPolicyLintCommandInput,
) -> anyhow::Result<()> {
    policy::run_policy_lint_command(input)
}

pub fn run_privileged_worker_command(
    input: crate::commands::input::PrivilegedWorkerCommandInput,
) -> anyhow::Result<()> {
    privileged_worker::run_privileged_worker_command(input)
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
    acceptance::run_acceptance_command(input)
}

pub fn run_pause_command(
    input: crate::commands::input::DaemonPauseCommandInput,
) -> anyhow::Result<()> {
    lifecycle::run_pause_command(input)
}

pub fn run_resume_command(
    input: crate::commands::input::DaemonResumeCommandInput,
) -> anyhow::Result<()> {
    lifecycle::run_resume_command(input)
}

pub fn run_restore_command(
    input: crate::commands::input::DaemonRestoreCommandInput,
) -> anyhow::Result<()> {
    restore::run_restore_command(input)
}

pub fn run_resync_state_command(
    input: crate::commands::input::DaemonResyncStateCommandInput,
) -> anyhow::Result<()> {
    resync::run_resync_state_command(input)
}

pub fn run_rollback_drill_command(
    input: crate::commands::input::DaemonRollbackDrillCommandInput,
) -> anyhow::Result<()> {
    doctor::run_rollback_drill_command(input)
}
