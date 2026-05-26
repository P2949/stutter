use std::{ffi::OsString, sync::Arc};

use crate::commands::input::AppCommand;

pub(crate) fn parse_monitor_config_for_phase15<const N: usize>(
    args: [&str; N],
) -> anyhow::Result<Arc<crate::config::model::MonitorConfig>> {
    // invariant: used only in tests, mutex is infallible
    let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
    match crate::cli::parse_app_command_from(args.iter().map(OsString::from))? {
        AppCommand::Monitor(input) => Ok(input.config.clone()),
        other => anyhow::bail!("expected AppCommand::Monitor, got {other:?}"),
    }
}
