use super::*;

#[derive(Args, Debug, Clone)]
pub struct ValidateArgs {
    #[arg(help = "Path to run directory or session.json")]
    pub path: PathBuf,

    #[arg(long)]
    pub json: bool,

    #[arg(long, help = "Treat warnings and medium-quality data as failures")]
    pub strict: bool,
}

pub fn validate_pids(flag: &str, pids: &[u32]) -> anyhow::Result<()> {
    if pids.contains(&0) {
        anyhow::bail!("{flag} must be greater than zero");
    }
    Ok(())
}

pub fn validate_comm_patterns(flag: &str, patterns: &[String]) -> anyhow::Result<()> {
    for pattern in patterns {
        if pattern.is_empty() {
            anyhow::bail!("{flag} patterns must not be empty");
        }
    }
    Ok(())
}

pub fn parse_optional_task_class(value: Option<&str>) -> anyhow::Result<Option<TaskClass>> {
    value
        .map(|s| {
            TaskClass::from_str_opt(s).ok_or_else(|| anyhow::anyhow!("unknown task class: {s}"))
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::commands::input::AppCommand;

    fn parse_validate_command<const N: usize>(args: [&str; N]) -> anyhow::Result<AppCommand> {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        crate::cli::parse_app_command_from(args)
    }

    #[test]
    fn validate_requires_path() {
        let err = parse_validate_command(["stutter", "validate"]).unwrap_err();

        assert!(
            err.to_string().contains("required")
                || err.to_string().contains("PATH")
                || err.to_string().contains("path"),
            "expected missing path error, got {err:#}"
        );
    }

    #[test]
    fn validate_accepts_path() {
        let command = parse_validate_command(["stutter", "validate", "/tmp/run"]).unwrap();

        let AppCommand::Validate(input) = command else {
            panic!("expected validate command");
        };

        assert_eq!(input.path, PathBuf::from("/tmp/run"));
        assert!(!input.json);
        assert!(!input.strict);
    }

    #[test]
    fn validate_accepts_json_flag() {
        let command =
            parse_validate_command(["stutter", "validate", "--json", "/tmp/run"]).unwrap();

        let AppCommand::Validate(input) = command else {
            panic!("expected validate command");
        };

        assert_eq!(input.path, PathBuf::from("/tmp/run"));
        assert!(input.json);
        assert!(!input.strict);
    }

    #[test]
    fn validate_accepts_strict_flag() {
        let command =
            parse_validate_command(["stutter", "validate", "--strict", "/tmp/run"]).unwrap();

        let AppCommand::Validate(input) = command else {
            panic!("expected validate command");
        };

        assert_eq!(input.path, PathBuf::from("/tmp/run"));
        assert!(!input.json);
        assert!(input.strict);
    }

    #[test]
    fn validate_accepts_json_and_strict_flags() {
        let command =
            parse_validate_command(["stutter", "validate", "--json", "--strict", "/tmp/run"])
                .unwrap();

        let AppCommand::Validate(input) = command else {
            panic!("expected validate command");
        };

        assert_eq!(input.path, PathBuf::from("/tmp/run"));
        assert!(input.json);
        assert!(input.strict);
    }
}
