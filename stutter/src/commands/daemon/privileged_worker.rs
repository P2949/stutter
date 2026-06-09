pub fn run_privileged_worker_command(
    input: crate::commands::input::PrivilegedWorkerCommandInput,
) -> anyhow::Result<()> {
    crate::daemon::privilege::run_privileged_worker(&input.socket)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn privileged_worker_command_runner_has_expected_signature() {
        let _runner: fn(
            crate::commands::input::PrivilegedWorkerCommandInput,
        ) -> anyhow::Result<()> = run_privileged_worker_command;
    }
}
