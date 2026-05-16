use super::*;

#[derive(Args, Debug, Clone)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum ConfigCommand {
    Check(ConfigCheckArgs),
    Explain(ConfigExplainArgs),
}

#[derive(Args, Debug, Clone)]
pub struct ConfigCheckArgs {
    #[arg(long = "json")]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct ConfigExplainArgs {
    #[arg(long = "json")]
    pub json: bool,

    #[arg(long = "preset", value_name = "NAME")]
    pub preset: Option<String>,
}

#[cfg(test)]
mod tests {
    use crate::commands::input::AppCommand;

    fn parse_config_command<const N: usize>(args: [&str; N]) -> anyhow::Result<AppCommand> {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        crate::cli::parse_app_command_from(args)
    }

    #[test]
    fn parses_config_check_command() {
        let command = parse_config_command(["stutter", "config", "check"]).unwrap();

        let AppCommand::ConfigCheck(input) = command else {
            panic!("expected config check command");
        };

        assert!(!input.json);
    }

    #[test]
    fn parses_config_check_json_command() {
        let command = parse_config_command(["stutter", "config", "check", "--json"]).unwrap();

        let AppCommand::ConfigCheck(input) = command else {
            panic!("expected config check command");
        };

        assert!(input.json);
    }

    #[test]
    fn parses_config_explain_command() {
        let command = parse_config_command(["stutter", "config", "explain"]).unwrap();

        let AppCommand::ConfigExplain(input) = command else {
            panic!("expected config explain command");
        };

        assert!(!input.json);
        assert_eq!(input.preset, None);
    }

    #[test]
    fn parses_config_explain_json_command() {
        let command = parse_config_command(["stutter", "config", "explain", "--json"]).unwrap();

        let AppCommand::ConfigExplain(input) = command else {
            panic!("expected config explain command");
        };

        assert!(input.json);
        assert_eq!(input.preset, None);
    }

    #[test]
    fn parses_config_explain_preset_command() {
        let command = parse_config_command([
            "stutter",
            "config",
            "explain",
            "--preset",
            "gaming-low-risk",
        ])
        .unwrap();

        let AppCommand::ConfigExplain(input) = command else {
            panic!("expected config explain command");
        };

        assert!(!input.json);
        assert_eq!(input.preset.as_deref(), Some("gaming-low-risk"));
    }

    #[test]
    fn parses_config_explain_json_and_preset_command() {
        let command = parse_config_command([
            "stutter",
            "config",
            "explain",
            "--json",
            "--preset",
            "gaming-low-risk",
        ])
        .unwrap();

        let AppCommand::ConfigExplain(input) = command else {
            panic!("expected config explain command");
        };

        assert!(input.json);
        assert_eq!(input.preset.as_deref(), Some("gaming-low-risk"));
    }
}
