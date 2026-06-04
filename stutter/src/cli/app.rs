use clap::{Parser, Subcommand};

use super::{
    agent::{AgentArgs, PrivilegedWorkerArgs},
    autotune::{AutotuneArgs, AutotuneStatusArgs},
    config::ConfigArgs,
    daemon::DaemonArgs,
    monitor::{BenchArgs, MonitorArgs, RecordArgs},
    prove_fix::ProveFixArgs,
    release::ReleaseArgs,
    report::{
        AdvisorArgs, ApplyProfileArgs, AuditArgs, CheckArgs, CompareArgs, CompletionsArgs,
        DoctorArgs, InspectDrmTracepointsArgs, InspectIrqsArgs, InspectTreeArgs, ManArgs,
        ProbesArgs, ProfilePlanArgs, ProfileTemplateArgs, RecommendArgs, ReportArgs, RestoreArgs,
        RulesArgs, ScenarioArgs, SummaryArgs, TuneArgs, WaylandProbeArgs,
    },
    service::ServiceArgs,
    validate::ValidateArgs,
};

#[derive(Parser, Debug)]
#[command(
    version = crate::metadata::build_version(),
    about = "Profile scheduler runnable latency for selected tasks"
)]
pub(super) struct Cli {
    #[command(subcommand)]
    pub(super) command: Option<Command>,

    #[command(flatten)]
    pub(super) legacy_monitor: MonitorArgs,
}

#[derive(Subcommand, Debug)]
pub(super) enum Command {
    Monitor(MonitorArgs),
    Record(RecordArgs),
    Bench(BenchArgs),
    InspectTree(InspectTreeArgs),
    Report(ReportArgs),
    Summary(SummaryArgs),
    Validate(ValidateArgs),
    Restore(RestoreArgs),
    ApplyProfile(ApplyProfileArgs),
    #[command(
        name = "profile-plan",
        about = "Explain profile targets for a process tree"
    )]
    ProfilePlan(ProfilePlanArgs),
    Tune(TuneArgs),
    Recommend(RecommendArgs),
    #[command(name = "prove-fix")]
    ProveFix(ProveFixArgs),
    Release(ReleaseArgs),
    Check(CheckArgs),
    Compare(CompareArgs),
    Config(ConfigArgs),
    Audit(AuditArgs),
    Advisor(AdvisorArgs),
    Doctor(DoctorArgs),
    ProfileTemplate(ProfileTemplateArgs),
    #[command(name = "inspect-irqs")]
    InspectIrqs(InspectIrqsArgs),
    #[command(name = "inspect-drm-tracepoints")]
    InspectDrmTracepoints(InspectDrmTracepointsArgs),
    #[command(name = "wayland-probe")]
    WaylandProbe(WaylandProbeArgs),
    Autotune(AutotuneArgs),
    #[command(name = "autotune-status")]
    AutotuneStatus(AutotuneStatusArgs),
    Agent(AgentArgs),
    #[command(name = "privileged-worker")]
    PrivilegedWorker(PrivilegedWorkerArgs),
    Daemon(DaemonArgs),
    Service(ServiceArgs),
    #[command(name = "completions")]
    Completions(CompletionsArgs),
    #[command(name = "man")]
    Man(ManArgs),
    Probes(ProbesArgs),
    Rules(RulesArgs),
    Scenario(ScenarioArgs),
}
