use super::super::autotune::{
    AutotuneArgs, AutotuneCommand, AutotuneStatusArgs, validate_autotune_mode,
};
use crate::commands::input::{
    AppCommand, AutotuneApplyCandidateCommandInput, AutotuneCommandInput as AutotuneCommandDto,
    AutotuneGenerateProfilesCommandInput, AutotuneReplayCommandInput,
    AutotuneReplayHistoryCommandInput, AutotuneRestoreCommandInput, AutotuneStatusCommandInput,
};

pub(super) fn parse_autotune_command(args: AutotuneArgs) -> anyhow::Result<AppCommand> {
    if let Some(cmd) = args.command {
        match cmd {
            AutotuneCommand::GenerateProfiles(args) => Ok(AppCommand::AutotuneGenerateProfiles(
                AutotuneGenerateProfilesCommandInput {
                    watch_process: args.watch_process,
                    out: args.out,
                    allow_cpus: args.allow_cpus,
                    deny_cpus: args.deny_cpus,
                    min_render_cpus: args.min_render_cpus,
                    min_game_cpus: args.min_game_cpus,
                    min_compositor_cpus: args.min_compositor_cpus,
                    min_background_cpus: args.min_background_cpus,
                },
            )),
            AutotuneCommand::ApplyCandidate(args) => Ok(AppCommand::AutotuneApplyCandidate(
                AutotuneApplyCandidateCommandInput {
                    candidate_json: args.candidate_json,
                    dry_run: args.dry_run,
                },
            )),
            AutotuneCommand::Replay(replay) => {
                Ok(AppCommand::AutotuneReplay(AutotuneReplayCommandInput {
                    run: replay.run,
                    config: replay.config,
                }))
            }
            AutotuneCommand::ReplayHistory(replay_args) => Ok(AppCommand::AutotuneReplayHistory(
                AutotuneReplayHistoryCommandInput {
                    history: replay_args.history,
                },
            )),
            AutotuneCommand::Restore(args) => {
                Ok(AppCommand::AutotuneRestore(AutotuneRestoreCommandInput {
                    journal: args.journal,
                    audit: args.audit,
                    history: args.history,
                    dry_run: args.dry_run,
                }))
            }
        }
    } else {
        validate_autotune_mode(args.mode, args.allow_medium_risk, args.dry_run_all_safe)?;
        Ok(AppCommand::Autotune(AutotuneCommandDto {
            input: crate::autotune::commands::live::AutotuneCommandInput {
                config: args.config,
                watch_process: args.watch_process,
                tree_pid: args.tree_pid,
                profiles: args.profiles,
                mode: args.mode.as_daemon_mode(),
                decision_log: args.decision_log,
                duration_seconds: args.duration_seconds,
                washout_seconds: args.washout_seconds,
                washout_verify_interval_ms: args.washout_verify_interval_ms,
                summary_ms: args.summary_ms,
                preset: args.preset,
                hwmon: args.hwmon,
                mangohud_log: args.mangohud_log,
                auto_focus: args.auto_focus,
                min_focus_confidence: args.min_focus_confidence,
                focus_source: args.focus_source,
                foreground_window: args.foreground_window,
                foreground_source: args.foreground_source,
                foreground_poll_ms: args.foreground_poll_ms,
                foreground_max_stale_ms: args.foreground_max_stale_ms,
                allow_system_wide_suggestions: args.allow_system_wide_suggestions,
                allow_medium_risk: args.allow_medium_risk,
                high_risk_dry_run: args.high_risk_dry_run,
                dry_run_all_safe: args.dry_run_all_safe,
            },
        }))
    }
}

pub(super) fn parse_autotune_status_command(
    args: AutotuneStatusArgs,
) -> anyhow::Result<AppCommand> {
    Ok(AppCommand::AutotuneStatus(AutotuneStatusCommandInput {
        json: args.json,
    }))
}
