use super::super::*;

#[cfg(test)]
mod rules_cli_tests {
    use super::super::*;

#[test]
    fn scenario_create_parses() {
        let command = parse_report_command([
            "stutter",
            "scenario",
            "create",
            "kcd-route",
            "--duration",
            "60",
            "--watch-process",
            "KingdomCome.exe",
            "--preset",
            "diagnosis",
            "--mangohud-log",
            "/tmp/mango.csv",
            "--notes",
            "forest route",
            "--force",
        ])
        .unwrap();

        let AppCommand::Scenario(input) = command else {
            panic!("expected scenario command");
        };
        let ScenarioCommandDto::Create(input) = input.command else {
            panic!("expected scenario create command");
        };

        assert_eq!(input.name, "kcd-route");
        assert!(input.force);
        assert_eq!(input.duration, 60);
        assert_eq!(input.watch_process, Some("KingdomCome.exe".to_owned()));
        assert_eq!(input.preset, "diagnosis");
        assert_eq!(input.mangohud_log, Some(PathBuf::from("/tmp/mango.csv")));
        assert_eq!(input.notes.as_deref(), Some("forest route"));
    }

#[test]
    fn scenario_run_parses_baseline() {
        let command = parse_report_command([
            "stutter",
            "scenario",
            "run",
            "kcd-route",
            "--role",
            "baseline",
            "--dry-run",
            "--out-dir",
            "/tmp/out",
            "--mangohud-log",
            "/tmp/override.csv",
        ])
        .unwrap();

        let AppCommand::Scenario(input) = command else {
            panic!("expected scenario command");
        };
        let ScenarioCommandDto::Run(input) = input.command else {
            panic!("expected scenario run command");
        };

        assert_eq!(input.name, "kcd-route");
        assert_eq!(input.role, "baseline");
        assert!(input.dry_run);
        assert_eq!(input.out_dir, Some(PathBuf::from("/tmp/out")));
        assert_eq!(
            input.mangohud_log_override,
            Some(PathBuf::from("/tmp/override.csv"))
        );
    }

#[test]
    fn scenario_run_parses_current() {
        let command = parse_report_command([
            "stutter",
            "scenario",
            "run",
            "kcd-route",
            "--role",
            "current",
        ])
        .unwrap();

        let AppCommand::Scenario(input) = command else {
            panic!("expected scenario command");
        };
        let ScenarioCommandDto::Run(input) = input.command else {
            panic!("expected scenario run command");
        };

        assert_eq!(input.name, "kcd-route");
        assert_eq!(input.role, "current");
    }

#[test]
    fn scenario_compare_parses() {
        let command = parse_report_command([
            "stutter",
            "scenario",
            "compare",
            "kcd-route",
            "--baseline",
            "/tmp/base",
            "--current",
            "/tmp/current",
            "--top",
            "5",
            "--json-summary",
            "--validate",
        ])
        .unwrap();

        let AppCommand::Scenario(input) = command else {
            panic!("expected scenario command");
        };
        let ScenarioCommandDto::Compare(input) = input.command else {
            panic!("expected scenario compare command");
        };

        assert_eq!(input.name, "kcd-route");
        assert_eq!(input.baseline, Some(PathBuf::from("/tmp/base")));
        assert_eq!(input.current, Some(PathBuf::from("/tmp/current")));
        assert_eq!(input.top, 5);
        assert!(input.json_summary);
        assert!(input.validate);
    }

#[test]
    fn scenario_path_and_list_parse() {
        let path = parse_report_command(["stutter", "scenario", "path", "kcd-route"]).unwrap();

        let AppCommand::Scenario(input) = path else {
            panic!("expected scenario command");
        };
        let ScenarioCommandDto::Path(input) = input.command else {
            panic!("expected scenario path command");
        };
        assert_eq!(input.name, "kcd-route");

        let list = parse_report_command(["stutter", "scenario", "list"]).unwrap();
        assert!(matches!(
            list,
            AppCommand::Scenario(crate::commands::input::ScenarioCommandInput {
                command: ScenarioCommandDto::List
            })
        ));
    }
}
