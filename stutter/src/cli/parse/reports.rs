use std::time::Duration;

use super::super::{
    config::{ConfigArgs, ConfigCommand},
    report::{
        AdvisorArgs, ApplyProfileArgs, AuditArgs, CheckArgs, CompareArgs, CompareCommand,
        CompletionsArgs, DoctorArgs, InspectDrmTracepointsArgs, InspectIrqsArgs, InspectTreeArgs,
        ManArgs, ProbesArgs, ProfileTemplateArgs, RecommendArgs, ReleaseArgs, ReleaseCommand,
        ReportArgs, RestoreArgs, SummaryArgs, TuneArgs, WaylandProbeArgs,
    },
    validate::{ValidateArgs, parse_optional_task_class},
};
use crate::{
    commands::input::{
        AdvisorCommandInput, AppCommand, ApplyProfileCommandInput, AuditCommandInput,
        CheckCommandInput, CompletionsCommandInput, ConfigCheckCommandInput,
        DaemonConfigExplainCommandInput, DisplayPathCompareCommandInput, DoctorCommandInput,
        InspectIrqsCommandInput, InspectTreeCommandInput, ManCommandInput, ProbesCommandInput,
        ProfileTemplateCommandInput, RecommendCommandInput, ReleaseCheckCommandInput,
        ReportCommandInput, RestoreCommandInput, SummaryCommandInput, TuneCommandInput,
        ValidateCommandInput, WaylandProbeCommandInput,
    },
    release::{ReleaseChannel, ReleaseReadinessInputs},
};

pub(super) fn parse_inspect_tree_command(args: InspectTreeArgs) -> anyhow::Result<AppCommand> {
    if args.tree_pid == 0 {
        anyhow::bail!("--tree-pid must be greater than zero");
    }
    Ok(AppCommand::InspectTree(InspectTreeCommandInput {
        tree_pid: args.tree_pid,
    }))
}

pub(super) fn parse_report_command(args: ReportArgs) -> anyhow::Result<AppCommand> {
    if args.top == 0 {
        anyhow::bail!("--top must be greater than zero");
    }
    if args.cluster_window_ms == 0 {
        anyhow::bail!("--cluster-ms must be greater than zero");
    }
    if args.batch.is_none() && args.path.is_none() {
        anyhow::bail!("report requires PATH unless --batch is set");
    }
    Ok(AppCommand::Report(ReportCommandInput {
        path: args.path,
        json: args.json,
        analysis_json: args.analysis_json,
        json_summary: args.json_summary,
        html: args.html,
        top: args.top,
        cluster_window_ms: args.cluster_window_ms,
        batch: args.batch,
        diff: args.diff,
        filter_class: parse_optional_task_class(args.filter_class.as_deref())?,
        flamegraph: args.flamegraph,
    }))
}

pub(super) fn parse_summary_command(args: SummaryArgs) -> anyhow::Result<AppCommand> {
    if args.top == 0 {
        anyhow::bail!("--top must be greater than zero");
    }
    Ok(AppCommand::Summary(SummaryCommandInput {
        path: args.path,
        json: args.json,
        top: args.top,
        filter_class: parse_optional_task_class(args.filter_class.as_deref())?,
    }))
}

pub(super) fn parse_validate_command(args: ValidateArgs) -> anyhow::Result<AppCommand> {
    Ok(AppCommand::Validate(ValidateCommandInput {
        path: args.path,
        json: args.json,
        strict: args.strict,
    }))
}

pub(super) fn parse_restore_command(args: RestoreArgs) -> anyhow::Result<AppCommand> {
    Ok(AppCommand::Restore(RestoreCommandInput {
        dry_run: args.dry_run,
    }))
}

pub(super) fn parse_apply_profile_command(args: ApplyProfileArgs) -> anyhow::Result<AppCommand> {
    if args.tree_pid == 0 {
        anyhow::bail!("--tree-pid must be greater than zero");
    }
    if args.refresh_ms == 0 {
        anyhow::bail!("--refresh-ms must be greater than zero");
    }
    if args.keep_applied && !args.watch {
        anyhow::bail!("--keep-applied requires --watch");
    }
    Ok(AppCommand::ApplyProfile(ApplyProfileCommandInput {
        tree_pid: args.tree_pid,
        profile: args.profile,
        force: args.force,
        dry_run: args.dry_run,
        allow_medium_risk: args.allow_medium_risk,
        watch: args.watch,
        keep_applied: args.keep_applied,
        refresh_ms: args.refresh_ms,
        enforce: args.enforce,
    }))
}

pub(super) fn parse_tune_command(args: TuneArgs) -> anyhow::Result<AppCommand> {
    if args.tree_pid == 0 {
        anyhow::bail!("--tree-pid must be greater than zero");
    }
    if args.epoch_seconds == 0 {
        anyhow::bail!("--epoch-seconds must be greater than zero");
    }
    if args.warmup_seconds >= args.epoch_seconds {
        anyhow::bail!("--warmup-seconds must be less than --epoch-seconds");
    }
    if args.runs == 0 {
        anyhow::bail!("--runs must be greater than zero");
    }
    Ok(AppCommand::Tune(TuneCommandInput {
        tree_pid: args.tree_pid,
        profiles: args.profiles,
        epoch_seconds: args.epoch_seconds,
        warmup_seconds: args.warmup_seconds,
        runs: args.runs,
        keep_best: args.keep_best,
        baseline_profile: args.baseline_profile,
        out_dir: args.out_dir,
        mangohud_log: args.mangohud_log,
        enforce: args.enforce,
        hwmon: args.hwmon,
    }))
}

pub(super) fn parse_recommend_command(args: RecommendArgs) -> anyhow::Result<AppCommand> {
    Ok(AppCommand::Recommend(RecommendCommandInput {
        baseline: args.baseline,
        tune: args.tune,
        json: args.json,
        markdown: args.markdown,
    }))
}

pub(super) fn parse_release_command(args: ReleaseArgs) -> anyhow::Result<AppCommand> {
    match args.command {
        ReleaseCommand::Check(args) => {
            let inputs = ReleaseReadinessInputs {
                apply_actions_enabled: args.apply_actions_enabled,
                soak_tests: args.soak_tests,
                stronger_tests: args.stronger_tests,
                real_machine_validation: args.real_machine_validation,
                production_distro_packaging: args.production_distro_packaging,
                reproducible_packaged_ebpf_object: args.reproducible_packaged_ebpf_object,
                packaging_install_tests: args.packaging_install_tests,
                packaging_service_smoke_tests: args.packaging_service_smoke_tests,
                versioned_release_tarball: args.versioned_release_tarball,
                ..ReleaseReadinessInputs::default()
            };
            Ok(AppCommand::ReleaseCheck(ReleaseCheckCommandInput {
                channel: args.channel.parse::<ReleaseChannel>()?,
                inputs,
                json: args.json,
                enforce: args.enforce,
            }))
        }
    }
}

pub(super) fn parse_check_command(args: CheckArgs) -> anyhow::Result<AppCommand> {
    if args.max_regression_p99_ms.is_none() && args.max_max_regression_ms.is_none() {
        anyhow::bail!(
            "check requires at least one threshold: --max-regression-p99-ms or --max-max-regression-ms"
        );
    }
    if let Some(value) = args.max_regression_p99_ms
        && (!value.is_finite() || value < 0.0)
    {
        anyhow::bail!("--max-regression-p99-ms must be a finite non-negative value");
    }
    if let Some(value) = args.max_max_regression_ms
        && (!value.is_finite() || value < 0.0)
    {
        anyhow::bail!("--max-max-regression-ms must be a finite non-negative value");
    }
    if args.top == 0 {
        anyhow::bail!("--top must be greater than zero");
    }
    Ok(AppCommand::Check(CheckCommandInput {
        baseline: args.baseline,
        current: args.current,
        max_regression_p99_ms: args.max_regression_p99_ms,
        max_max_regression_ms: args.max_max_regression_ms,
        json: args.json,
        top: args.top,
        filter_class: parse_optional_task_class(args.filter_class.as_deref())?,
    }))
}

pub(super) fn parse_config_command(args: ConfigArgs) -> anyhow::Result<AppCommand> {
    match args.command {
        ConfigCommand::Check(check_args) => Ok(AppCommand::ConfigCheck(ConfigCheckCommandInput {
            json: check_args.json,
        })),
        ConfigCommand::Explain(explain_args) => {
            Ok(AppCommand::ConfigExplain(DaemonConfigExplainCommandInput {
                json: explain_args.json,
                preset: explain_args.preset,
            }))
        }
    }
}

pub(super) fn parse_audit_command(args: AuditArgs) -> anyhow::Result<AppCommand> {
    Ok(AppCommand::Audit(AuditCommandInput {
        path: args.path,
        tail: args.tail,
        json: args.json,
    }))
}

pub(super) fn parse_advisor_command(args: AdvisorArgs) -> anyhow::Result<AppCommand> {
    if args.watch_runs && args.run.is_some() {
        anyhow::bail!("--watch-runs conflicts with --run");
    }
    if !args.watch_runs && args.run.is_none() {
        anyhow::bail!("advisor requires --run unless --watch-runs is set");
    }
    if args.poll_seconds == 0 {
        anyhow::bail!("--poll-seconds must be greater than zero");
    }
    Ok(AppCommand::Advisor(AdvisorCommandInput {
        run: args.run,
        profiles: args.profiles,
        json: args.json,
        watch_runs: args.watch_runs,
        runs_dir: args.runs_dir,
        poll_seconds: args.poll_seconds,
        once: args.once,
    }))
}

pub(super) fn parse_doctor_command(args: DoctorArgs) -> anyhow::Result<AppCommand> {
    Ok(AppCommand::Doctor(DoctorCommandInput {
        input: args.into_input()?,
    }))
}

pub(super) fn parse_profile_template_command(
    args: ProfileTemplateArgs,
) -> anyhow::Result<AppCommand> {
    Ok(AppCommand::ProfileTemplate(ProfileTemplateCommandInput {
        topology: args.topology,
    }))
}

pub(super) fn parse_inspect_irqs_command(args: InspectIrqsArgs) -> anyhow::Result<AppCommand> {
    if args.top == 0 {
        anyhow::bail!("--top must be greater than zero");
    }
    Ok(AppCommand::InspectIrqs(InspectIrqsCommandInput {
        json: args.json,
        filter: args.filter.clone(),
        top: args.top,
    }))
}

pub(super) fn parse_inspect_drm_tracepoints_command(
    args: InspectDrmTracepointsArgs,
) -> anyhow::Result<AppCommand> {
    Ok(AppCommand::InspectDrmTracepoints(
        crate::commands::input::InspectDrmTracepointsCommandInput {
            json: args.json,
            events_root: args.events_root,
        },
    ))
}

pub(super) fn parse_compare_command(args: CompareArgs) -> anyhow::Result<AppCommand> {
    match args.command {
        CompareCommand::DisplayPath(display) => Ok(AppCommand::DisplayPathCompare(
            DisplayPathCompareCommandInput {
                baseline: display.baseline.clone(),
                test: display.test.clone(),
                json: display.json,
                strict: display.strict,
                expect: display.expect,
            },
        )),
    }
}

pub(super) fn parse_wayland_probe_command(args: WaylandProbeArgs) -> anyhow::Result<AppCommand> {
    if args.duration_secs == 0 {
        anyhow::bail!("--duration must be greater than zero");
    }
    Ok(AppCommand::WaylandProbe(WaylandProbeCommandInput {
        duration: Duration::from_secs(args.duration_secs),
        output: args.output.clone(),
        fullscreen: args.fullscreen,
        out_dir: args.out_dir.clone(),
    }))
}

pub(super) fn parse_completions_command(args: CompletionsArgs) -> anyhow::Result<AppCommand> {
    Ok(AppCommand::Completions(CompletionsCommandInput {
        shell: args.shell,
    }))
}

pub(super) fn parse_man_command(args: ManArgs) -> anyhow::Result<AppCommand> {
    Ok(AppCommand::Man(ManCommandInput {
        output: args.output,
    }))
}

pub(super) fn parse_probes_command(args: ProbesArgs) -> anyhow::Result<AppCommand> {
    Ok(AppCommand::Probes(ProbesCommandInput {
        json: args.json,
        include_planned: args.include_planned,
    }))
}

#[cfg(test)]
mod probes_tests {
    use crate::{cli::parse_app_command_from, commands::input::AppCommand};

    #[test]
    fn probes_include_planned_flag_is_parsed() {
        let command = parse_app_command_from(["stutter", "probes", "--include-planned"])
            .expect("probes --include-planned should parse");

        match command {
            AppCommand::Probes(input) => {
                assert!(!input.json);
                assert!(input.include_planned);
            }
            other => panic!("expected probes command, got {other:?}"),
        }
    }

    #[test]
    fn probes_json_include_planned_flags_are_parsed_together() {
        let command = parse_app_command_from(["stutter", "probes", "--json", "--include-planned"])
            .expect("probes --json --include-planned should parse");

        match command {
            AppCommand::Probes(input) => {
                assert!(input.json);
                assert!(input.include_planned);
            }
            other => panic!("expected probes command, got {other:?}"),
        }
    }

    #[test]
    fn probes_default_hides_planned_flag_is_false() {
        let command = parse_app_command_from(["stutter", "probes"]).expect("probes should parse");

        match command {
            AppCommand::Probes(input) => {
                assert!(!input.json);
                assert!(!input.include_planned);
            }
            other => panic!("expected probes command, got {other:?}"),
        }
    }
}
