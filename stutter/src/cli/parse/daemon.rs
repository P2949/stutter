use super::super::daemon::{DaemonArgs, DaemonCommand, DaemonPolicyCommand, DaemonProfilesCommand};
use crate::{
    commands::input::{
        AppCommand, DaemonAcceptanceCommandInput, DaemonBenchOverheadCommandInput,
        DaemonConfigExplainCommandInput, DaemonDoctorCommandInput, DaemonExplainCommandInput,
        DaemonPauseCommandInput, DaemonPolicyExplainCommandInput, DaemonPolicyLintCommandInput,
        DaemonProfilesCommandInput, DaemonProfilesExplainCommandInput,
        DaemonProfilesForgetCommandInput, DaemonProfilesListCommandInput,
        DaemonResetStateCommandInput, DaemonRestoreCommandInput, DaemonResumeCommandInput,
        DaemonResyncStateCommandInput, DaemonSoakCommandInput, DaemonStatusCommandInput,
        DaemonWatchCommandInput, DaemonWhatChangedCommandInput, DaemonWhyNotOptimizeCommandInput,
    },
    daemon::testing::{DaemonSoakBudget, DaemonSoakConfig, DaemonSoakProfile},
};

pub(super) fn parse_daemon_command(args: DaemonArgs) -> anyhow::Result<AppCommand> {
    match args.command {
        DaemonCommand::Config(config_args) => {
            if !config_args.explain {
                anyhow::bail!("daemon config requires --explain");
            }
            Ok(AppCommand::DaemonConfigExplain(
                DaemonConfigExplainCommandInput {
                    json: config_args.json,
                    preset: config_args.preset,
                },
            ))
        }
        DaemonCommand::Policy(policy_args) => match policy_args.command {
            DaemonPolicyCommand::Explain(explain_args) => Ok(AppCommand::DaemonPolicyExplain(
                DaemonPolicyExplainCommandInput {
                    json: explain_args.json,
                    preset: explain_args.preset,
                },
            )),
        },
        DaemonCommand::PolicyLint(lint_args) => {
            Ok(AppCommand::DaemonPolicyLint(DaemonPolicyLintCommandInput {
                json: lint_args.json,
                preset: lint_args.preset,
            }))
        }
        DaemonCommand::Profiles(profiles_args) => match profiles_args.command {
            DaemonProfilesCommand::List(list_args) => Ok(AppCommand::DaemonProfiles(
                DaemonProfilesCommandInput::List(DaemonProfilesListCommandInput {
                    json: list_args.json,
                }),
            )),
            DaemonProfilesCommand::Forget(forget_args) => {
                if !forget_args.all && forget_args.workload_identity_hash.is_none() {
                    anyhow::bail!("daemon profiles forget requires --workload-hash or --all");
                }
                if forget_args.all && forget_args.workload_identity_hash.is_some() {
                    anyhow::bail!("--all conflicts with --workload-hash");
                }
                Ok(AppCommand::DaemonProfiles(
                    DaemonProfilesCommandInput::Forget(DaemonProfilesForgetCommandInput {
                        workload_identity_hash: forget_args.workload_identity_hash,
                        candidate: forget_args.candidate,
                        all: forget_args.all,
                        dry_run: forget_args.dry_run,
                        json: forget_args.json,
                    }),
                ))
            }
            DaemonProfilesCommand::Explain(explain_args) => Ok(AppCommand::DaemonProfiles(
                DaemonProfilesCommandInput::Explain(DaemonProfilesExplainCommandInput {
                    workload_identity_hash: explain_args.workload_identity_hash,
                    json: explain_args.json,
                }),
            )),
        },
        DaemonCommand::Explain(explain_args) => {
            Ok(AppCommand::DaemonExplain(DaemonExplainCommandInput {
                json: explain_args.json,
                explain_last: explain_args.explain_last,
            }))
        }
        DaemonCommand::WhyNotOptimize(args) => Ok(AppCommand::DaemonWhyNotOptimize(
            DaemonWhyNotOptimizeCommandInput {
                json: args.json,
                explain_last: args.explain_last,
            },
        )),
        DaemonCommand::WhatChanged(args) => Ok(AppCommand::DaemonWhatChanged(
            DaemonWhatChangedCommandInput {
                json: args.json,
                explain_last: args.explain_last,
            },
        )),
        DaemonCommand::Status(status_args) => {
            Ok(AppCommand::DaemonStatus(DaemonStatusCommandInput {
                json: status_args.json,
                explain_last: status_args.explain_last,
            }))
        }
        DaemonCommand::Watch(watch_args) => {
            if watch_args.interval_ms == 0 {
                anyhow::bail!("--interval-ms must be greater than zero");
            }
            if watch_args.iterations == Some(0) {
                anyhow::bail!("--iterations must be greater than zero");
            }
            Ok(AppCommand::DaemonWatch(DaemonWatchCommandInput {
                interval_ms: watch_args.interval_ms,
                iterations: watch_args.iterations,
                verbose: watch_args.verbose,
                explain_last: watch_args.explain_last,
            }))
        }
        DaemonCommand::Doctor(doctor_args) => {
            Ok(AppCommand::DaemonDoctor(DaemonDoctorCommandInput {
                json: doctor_args.json,
            }))
        }
        DaemonCommand::ResetState(reset_args) => {
            Ok(AppCommand::DaemonResetState(DaemonResetStateCommandInput {
                dry_run: reset_args.dry_run,
                json: reset_args.json,
            }))
        }
        DaemonCommand::BenchOverhead(bench_args) => Ok(AppCommand::DaemonBenchOverhead(
            DaemonBenchOverheadCommandInput {
                json: bench_args.json,
                duration_ms: bench_args.duration_ms,
            },
        )),
        DaemonCommand::Soak(soak_args) => {
            if soak_args.duration_seconds == 0 {
                anyhow::bail!("--duration-seconds must be greater than zero");
            }
            if soak_args.tick_ms == 0 {
                anyhow::bail!("--tick-ms must be greater than zero");
            }
            let mut budget = DaemonSoakBudget::default();
            if let Some(max_disk_growth_bytes) = soak_args.max_disk_growth_bytes {
                budget.max_disk_growth_bytes = max_disk_growth_bytes;
            }
            Ok(AppCommand::DaemonSoak(DaemonSoakCommandInput {
                config: DaemonSoakConfig {
                    profile: soak_args.profile.parse::<DaemonSoakProfile>()?,
                    duration_seconds: soak_args.duration_seconds,
                    tick_millis: soak_args.tick_ms,
                    budget,
                },
                json: soak_args.json,
            }))
        }
        DaemonCommand::Acceptance(acceptance_args) => {
            Ok(AppCommand::DaemonAcceptance(DaemonAcceptanceCommandInput {
                json: acceptance_args.json,
            }))
        }
        DaemonCommand::Pause(_) => Ok(AppCommand::DaemonPause(DaemonPauseCommandInput)),
        DaemonCommand::Resume(_) => Ok(AppCommand::DaemonResume(DaemonResumeCommandInput)),
        DaemonCommand::ResyncState(resync_args) => Ok(AppCommand::DaemonResyncState(
            DaemonResyncStateCommandInput {
                dry_run: resync_args.dry_run,
                json: resync_args.json,
            },
        )),
        DaemonCommand::Restore(restore_args) => {
            Ok(AppCommand::DaemonRestore(DaemonRestoreCommandInput {
                dry_run: restore_args.dry_run,
                emergency: false,
            }))
        }
        DaemonCommand::EmergencyRestore(restore_args) => {
            Ok(AppCommand::DaemonRestore(DaemonRestoreCommandInput {
                dry_run: restore_args.dry_run,
                emergency: true,
            }))
        }
    }
}
