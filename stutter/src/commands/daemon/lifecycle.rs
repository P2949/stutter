use crate::daemon::state::default_daemon_state_snapshot_path;

pub fn run_pause_command(_: crate::commands::input::DaemonPauseCommandInput) -> anyhow::Result<()> {
    let state_path = default_daemon_state_snapshot_path();
    let mut store = super::helpers::daemon_state_store_for_path(&state_path)?;

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
    let mut store = super::helpers::daemon_state_store_for_path(&state_path)?;

    store.resume("operator requested daemon resume")?;

    println!("daemon resumed; state_path={}", state_path.display());
    Ok(())
}
