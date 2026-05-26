use crate::commands::input::AppCommand;

pub(super) fn parse_report_command<const N: usize>(args: [&str; N]) -> anyhow::Result<AppCommand> {
    let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
    crate::cli::parse_app_command_from(args)
}
