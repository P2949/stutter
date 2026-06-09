use super::super::report::{RulesArgs, RulesCommand, ScenarioArgs, ScenarioCommand};
use crate::commands::input::{
    AppCommand, RulesCommandInput, ScenarioCompareCommandInput, ScenarioCreateCommandInput,
    ScenarioPathCommandInput, ScenarioRunCommandInput,
};

pub(super) fn parse_rules_command(args: RulesArgs) -> anyhow::Result<AppCommand> {
    Ok(AppCommand::Rules(RulesCommandInput {
        command: match args.command {
            RulesCommand::Import(args) => crate::commands::input::RulesCommand::Import(args.into()),
            RulesCommand::Check(args) => crate::commands::input::RulesCommand::Check(args.into()),
            RulesCommand::List(_) => crate::commands::input::RulesCommand::List,
            RulesCommand::Status(_) => crate::commands::input::RulesCommand::Status,
            RulesCommand::Enable(args) => crate::commands::input::RulesCommand::Enable(
                crate::commands::input::RulesEnableArgs { name: args.name },
            ),
            RulesCommand::Disable(_) => crate::commands::input::RulesCommand::Disable,
            RulesCommand::Remove(args) => crate::commands::input::RulesCommand::Remove(
                crate::commands::input::RulesRemoveArgs {
                    name: args.name,
                    dry_run: args.dry_run,
                },
            ),
        },
    }))
}

pub(super) fn parse_scenario_command(args: ScenarioArgs) -> anyhow::Result<AppCommand> {
    Ok(AppCommand::Scenario(
        crate::commands::input::ScenarioCommandInput {
            command: match args.command {
                ScenarioCommand::Create(args) => {
                    if args.name.trim().is_empty() {
                        anyhow::bail!("scenario name must not be empty");
                    }
                    if args.duration == 0 {
                        anyhow::bail!("scenario duration must be greater than zero");
                    }
                    crate::commands::input::ScenarioCommand::Create(ScenarioCreateCommandInput {
                        name: args.name,
                        force: args.force,
                        watch_process: args.watch_process,
                        duration: args.duration,
                        preset: args.preset,
                        mangohud_log: args.mangohud_log,
                        notes: args.notes,
                    })
                }
                ScenarioCommand::Run(args) => {
                    if args.name.trim().is_empty() {
                        anyhow::bail!("scenario name must not be empty");
                    }
                    if !matches!(args.role.as_str(), "baseline" | "current") {
                        anyhow::bail!("--role must be baseline or current");
                    }
                    crate::commands::input::ScenarioCommand::Run(ScenarioRunCommandInput {
                        name: args.name,
                        role: args.role,
                        dry_run: args.dry_run,
                        out_dir: args.out_dir,
                        mangohud_log_override: args.mangohud_log_override,
                    })
                }
                ScenarioCommand::Compare(args) => {
                    if args.name.trim().is_empty() {
                        anyhow::bail!("scenario name must not be empty");
                    }
                    if args.top == 0 {
                        anyhow::bail!("--top must be greater than zero");
                    }
                    crate::commands::input::ScenarioCommand::Compare(ScenarioCompareCommandInput {
                        name: args.name,
                        baseline: args.baseline,
                        current: args.current,
                        top: args.top,
                        json_summary: args.json_summary,
                        validate: args.validate,
                    })
                }
                ScenarioCommand::Path(args) => {
                    if args.name.trim().is_empty() {
                        anyhow::bail!("scenario name must not be empty");
                    }
                    crate::commands::input::ScenarioCommand::Path(ScenarioPathCommandInput {
                        name: args.name,
                    })
                }
                ScenarioCommand::List => crate::commands::input::ScenarioCommand::List,
            },
        },
    ))
}
