use clap::{Parser, Subcommand};

use super::{
    agent::{AgentArgs, PrivilegedWorkerArgs},
    autotune::{AutotuneArgs, AutotuneStatusArgs},
    config::ConfigArgs,
    daemon::DaemonArgs,
    doctor::DoctorArgs,
    monitor::{BenchArgs, MonitorArgs, RecordArgs},
    prove_fix::ProveFixArgs,
    release::ReleaseArgs,
    report::{
        AdvisorArgs, ApplyProfileArgs, AuditArgs, CheckArgs, CompareArgs, CompletionsArgs,
        InspectDrmTracepointsArgs, InspectIrqsArgs, InspectTreeArgs, ManArgs, ProbesArgs,
        ProfilePlanArgs, ProfileTemplateArgs, RecommendArgs, ReportArgs, RestoreArgs, RulesArgs,
        ScenarioArgs, SummaryArgs, TuneArgs, WaylandProbeArgs,
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
    #[command(about = "Run live scheduler/frame monitoring for selected tasks")]
    Monitor(MonitorArgs),
    #[command(about = "Record scheduler/frame evidence for a target process tree")]
    Record(RecordArgs),
    #[command(about = "Benchmark a recording workflow against a target process tree")]
    Bench(BenchArgs),
    #[command(about = "Inspect a process tree before recording or profile planning")]
    InspectTree(InspectTreeArgs),
    #[command(about = "Generate an offline report from a recording")]
    Report(ReportArgs),
    #[command(about = "Print a compact summary for one or more recordings")]
    Summary(SummaryArgs),
    #[command(about = "Validate a recording directory and its artifacts")]
    Validate(ValidateArgs),
    #[command(about = "Restore affinity state captured by stutter")]
    Restore(RestoreArgs),
    #[command(about = "Apply or explain a scoped CPU-affinity profile")]
    ApplyProfile(ApplyProfileArgs),
    #[command(
        name = "profile-plan",
        about = "Explain profile targets for a process tree"
    )]
    ProfilePlan(ProfilePlanArgs),
    #[command(about = "Benchmark candidate profiles and select a supported result")]
    Tune(TuneArgs),
    #[command(about = "Compare baseline and tuned evidence before recommending")]
    Recommend(RecommendArgs),
    #[command(
        name = "prove-fix",
        about = "Print a guided workflow for proving a fix plan"
    )]
    ProveFix(ProveFixArgs),
    #[command(about = "Run release-readiness checks and related helpers")]
    Release(ReleaseArgs),
    #[command(about = "Check a current recording against a baseline recording")]
    Check(CheckArgs),
    #[command(about = "Compare specialized evidence paths between recordings")]
    Compare(CompareArgs),
    #[command(about = "Inspect and explain stutter configuration")]
    Config(ConfigArgs),
    #[command(about = "Inspect local audit events from tuning actions")]
    Audit(AuditArgs),
    #[command(about = "Suggest scoped fix plans from existing evidence")]
    Advisor(AdvisorArgs),
    #[command(about = "Run local preflight checks for stutter and eBPF support")]
    Doctor(DoctorArgs),
    #[command(about = "Create example CPU-affinity profile templates")]
    ProfileTemplate(ProfileTemplateArgs),
    #[command(
        name = "inspect-irqs",
        about = "Inspect IRQ distribution and candidate affinity targets"
    )]
    InspectIrqs(InspectIrqsArgs),
    #[command(
        name = "inspect-drm-tracepoints",
        about = "Inspect available DRM tracepoints for display-path evidence"
    )]
    InspectDrmTracepoints(InspectDrmTracepointsArgs),
    #[command(
        name = "wayland-probe",
        about = "Probe Wayland presentation timing support"
    )]
    WaylandProbe(WaylandProbeArgs),
    #[command(about = "Manage experimental autotune planning and replay commands")]
    Autotune(AutotuneArgs),
    #[command(
        name = "autotune-status",
        about = "Inspect experimental autotune controller status"
    )]
    AutotuneStatus(AutotuneStatusArgs),
    #[command(about = "Run experimental remote agent endpoints")]
    Agent(AgentArgs),
    #[command(
        name = "privileged-worker",
        about = "Run the privileged worker used by service paths"
    )]
    PrivilegedWorker(PrivilegedWorkerArgs),
    #[command(about = "Manage experimental daemon/service components")]
    Daemon(DaemonArgs),
    #[command(about = "Install, uninstall, or inspect service integration")]
    Service(ServiceArgs),
    #[command(name = "completions", about = "Generate shell completion scripts")]
    Completions(CompletionsArgs),
    #[command(name = "man", about = "Generate a stutter manual page")]
    Man(ManArgs),
    #[command(about = "List implemented and planned probe signals")]
    Probes(ProbesArgs),
    #[command(about = "Manage imported community rule metadata")]
    Rules(RulesArgs),
    #[command(about = "Create and compare repeatable benchmark scenarios")]
    Scenario(ScenarioArgs),
}
