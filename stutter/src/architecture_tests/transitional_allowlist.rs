//! Inventory of temporary migration markers; scanner policy lives in `transitional`.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::architecture_tests) struct MigrationModuleAllowance {
    pub(in crate::architecture_tests) path: &'static str,
    pub(in crate::architecture_tests) reason: &'static str,
    pub(in crate::architecture_tests) exit_criteria: &'static str,
}

macro_rules! migration_module {
    ($path:literal, $reason:literal, $exit_criteria:literal) => {
        MigrationModuleAllowance {
            path: $path,
            reason: $reason,
            exit_criteria: $exit_criteria,
        }
    };
}

pub(in crate::architecture_tests) const MAX_MIGRATION_MARKER_MODULES: usize = 71;

pub(in crate::architecture_tests) const MIGRATION_MODULE_ALLOWLIST: &[MigrationModuleAllowance] = &[
    migration_module!(
        "src/artifacts/compat_v20.rs",
        "legacy v20 artifact compatibility namespace is kept while readers support historical snapshots",
        "remove once v20 compatibility is handled at the loader boundary or support is retired"
    ),
    migration_module!(
        "src/artifacts/compat_v21.rs",
        "legacy v21 artifact compatibility namespace is kept while readers support historical snapshots",
        "remove once v21 compatibility is handled at the loader boundary or support is retired"
    ),
    migration_module!(
        "src/autotune/domain/mod.rs",
        "autotune domain facade keeps pure type extraction stable during import migration",
        "remove once callers import the domain submodules directly and no facade imports remain"
    ),
    migration_module!(
        "src/autotune/human_output.rs",
        "human output renderer remains available while callers migrate through autotune::output",
        "remove once all presentation call sites use the final output module boundary"
    ),
    migration_module!(
        "src/autotune/observation/builder.rs",
        "observation builder facade preserves old paths during observation extraction",
        "remove once old observation builder paths are unused"
    ),
    migration_module!(
        "src/autotune/observation/observation.rs",
        "observation facade preserves old paths during observation extraction",
        "remove once old observation paths are unused"
    ),
    migration_module!(
        "src/autotune/observation/quality.rs",
        "quality facade preserves old paths during observation extraction",
        "remove once old quality paths are unused"
    ),
    migration_module!(
        "src/autotune/observation/rolling_window.rs",
        "rolling-window facade preserves old paths during observation extraction",
        "remove once old rolling-window paths are unused"
    ),
    migration_module!(
        "src/autotune/observation/system_context.rs",
        "system-context facade preserves old paths during observation extraction",
        "remove once old system-context paths are unused"
    ),
    migration_module!(
        "src/autotune/observation/target_selection.rs",
        "target-selection facade preserves old paths during observation extraction",
        "remove once old target-selection paths are unused"
    ),
    migration_module!(
        "src/autotune/output/mod.rs",
        "output facade keeps presentation module movement source-compatible",
        "remove once callers import concrete output modules directly"
    ),
    migration_module!(
        "src/autotune/planning/candidate.rs",
        "planning candidate wrapper remains while providers migrate from CandidateAction",
        "remove once providers emit the final planning candidate type directly"
    ),
    migration_module!(
        "src/autotune/runtime/controller.rs",
        "runtime controller facade preserves old paths during runtime split",
        "remove once old runtime controller paths are unused"
    ),
    migration_module!(
        "src/autotune/runtime/emergency_restore.rs",
        "runtime emergency-restore facade preserves old paths during runtime split",
        "remove once old runtime emergency-restore paths are unused"
    ),
    migration_module!(
        "src/autotune/runtime/journal.rs",
        "runtime journal facade preserves old paths during runtime split",
        "remove once old runtime journal paths are unused"
    ),
    migration_module!(
        "src/autotune/runtime/shutdown.rs",
        "runtime shutdown facade preserves old paths during runtime split",
        "remove once old runtime shutdown paths are unused"
    ),
    migration_module!(
        "src/autotune/runtime/startup_recovery.rs",
        "runtime startup-recovery facade preserves old paths during runtime split",
        "remove once old runtime startup-recovery paths are unused"
    ),
    migration_module!(
        "src/cli/map/autotune.rs",
        "CLI command-family mapping placeholder keeps the staged parser split compiling",
        "remove once autotune command mapping has moved from cli/mod.rs"
    ),
    migration_module!(
        "src/cli/map/daemon.rs",
        "CLI command-family mapping placeholder keeps the staged parser split compiling",
        "remove once daemon command mapping has moved from cli/mod.rs"
    ),
    migration_module!(
        "src/cli/map/mod.rs",
        "CLI command-family mapping namespace keeps the staged parser split compiling",
        "remove once command-family mapping has moved from cli/mod.rs"
    ),
    migration_module!(
        "src/cli/map/monitor.rs",
        "CLI command-family mapping placeholder keeps the staged parser split compiling",
        "remove once monitor command mapping has moved from cli/mod.rs"
    ),
    migration_module!(
        "src/cli/map/report.rs",
        "CLI command-family mapping placeholder keeps the staged parser split compiling",
        "remove once report command mapping has moved from cli/mod.rs"
    ),
    migration_module!(
        "src/cli/map/rules.rs",
        "CLI command-family mapping placeholder keeps the staged parser split compiling",
        "remove once rules command mapping has moved from cli/mod.rs"
    ),
    migration_module!(
        "src/cli/map/service.rs",
        "CLI command-family mapping placeholder keeps the staged parser split compiling",
        "remove once service command mapping has moved from cli/mod.rs"
    ),
    migration_module!(
        "src/cli/parse.rs",
        "CLI parse facade remains while parse_app_command migrates from cli/mod.rs",
        "remove once parsing entry points live fully in cli/parse.rs"
    ),
    migration_module!(
        "src/cli/version_parse.rs",
        "CLI version parsing target exists while version parsing moves incrementally",
        "remove once version parsing is owned here and used by callers"
    ),
    migration_module!(
        "src/community_rules/import/mod.rs",
        "community-rule import namespace wraps the existing importer during import split",
        "remove once importer call sites use the final import modules directly"
    ),
    migration_module!(
        "src/community_rules/import/parse.rs",
        "community-rule import parser extraction target exists during Ananicy input split",
        "remove once parsing is fully owned here and used by the importer"
    ),
    migration_module!(
        "src/community_rules/import/report.rs",
        "community-rule import report facade keeps result types stable during split",
        "remove once report types live here without re-export compatibility"
    ),
    migration_module!(
        "src/community_rules/import/validate.rs",
        "community-rule import validation extraction target exists during import split",
        "remove once validation is fully owned here and used by the importer"
    ),
    migration_module!(
        "src/daemon/policy/explain.rs",
        "daemon policy explanation facade preserves old paths during policy split",
        "remove once callers import final policy explanation modules directly"
    ),
    migration_module!(
        "src/daemon/state_compat.rs",
        "daemon state compatibility namespace remains during state model migration",
        "remove once daemon state callers use the canonical model"
    ),
    migration_module!(
        "src/ebpf/attach.rs",
        "eBPF attach target exists while attachment logic migrates from ebpf_loader",
        "remove once attach logic is fully owned here and old loader paths are unused"
    ),
    migration_module!(
        "src/ebpf/capabilities.rs",
        "eBPF capability target exists while probing migrates from ebpf_loader",
        "remove once capability probing is fully owned here and old loader paths are unused"
    ),
    migration_module!(
        "src/ebpf/maps.rs",
        "eBPF map target exists while map setup migrates from ebpf_loader",
        "remove once map setup is fully owned here and old loader paths are unused"
    ),
    migration_module!(
        "src/ebpf/object.rs",
        "eBPF object target exists while object loading migrates from ebpf_loader",
        "remove once object loading is fully owned here and old loader paths are unused"
    ),
    migration_module!(
        "src/ebpf/ringbuf.rs",
        "eBPF ring/perf buffer target exists while setup migrates from ebpf_loader",
        "remove once ring/perf setup is fully owned here and old loader paths are unused"
    ),
    migration_module!(
        "src/ebpf/tracepoint_format.rs",
        "eBPF tracepoint validation target exists while validation migrates from ebpf_loader",
        "remove once tracepoint validation is fully owned here and old loader paths are unused"
    ),
    migration_module!(
        "src/events/domain.rs",
        "event domain wrappers remain while raw decoders migrate path by path",
        "remove once callers use final event domain modules directly"
    ),
    migration_module!(
        "src/foreground/command.rs",
        "foreground command-runner injection target exists during foreground split",
        "remove once command execution is fully injected through this module"
    ),
    migration_module!(
        "src/process/mod.rs",
        "process facade preserves old imports while process_tree splits",
        "remove once callers import concrete process modules directly"
    ),
    migration_module!(
        "src/process/procfs.rs",
        "procfs reader trait remains while process-tree I/O splits",
        "remove once process-tree I/O uses the final procfs boundary"
    ),
    migration_module!(
        "src/profiles/apply.rs",
        "profile apply facade preserves old imports during profile split",
        "remove once profile application callers target the final module boundary"
    ),
    migration_module!(
        "src/profiles/cache.rs",
        "profile cache facade preserves old imports during profile split",
        "remove once profile cache callers target the final module boundary"
    ),
    migration_module!(
        "src/profiles/matcher.rs",
        "profile matcher facade preserves old imports during profile split",
        "remove once profile matcher callers target the final module boundary"
    ),
    migration_module!(
        "src/profiles/model.rs",
        "profile model facade preserves old imports during profile split",
        "remove once profile model callers target the final module boundary"
    ),
    migration_module!(
        "src/profiles/plan.rs",
        "profile plan facade preserves old imports during profile split",
        "remove once profile planning callers target the final module boundary"
    ),
    migration_module!(
        "src/profiles/validate.rs",
        "profile validation extraction target exists during profile split",
        "remove once profile validation callers target the final module boundary"
    ),
    migration_module!(
        "src/profiles/verify.rs",
        "profile verify extraction target exists during profile split",
        "remove once profile verification callers target the final module boundary"
    ),
    migration_module!(
        "src/remote/compat.rs",
        "remote compatibility namespace keeps remote API paths stable during split",
        "remove once remote API callers use canonical modules directly"
    ),
    migration_module!(
        "src/schemas/mod.rs",
        "schema facade preserves serialized model imports while schemas move from owners",
        "remove once callers import concrete schema modules directly"
    ),
    migration_module!(
        "src/service/autotune.rs",
        "service boundary exists while agent and CLI autotune call sites migrate incrementally",
        "remove once autotune service callers use the final service boundary"
    ),
    migration_module!(
        "src/service/community_rules.rs",
        "service boundary exists while community-rule command call sites migrate incrementally",
        "remove once community-rule callers use the final service boundary"
    ),
    migration_module!(
        "src/service/daemon.rs",
        "service boundary exists while agent and CLI daemon call sites migrate incrementally",
        "remove once daemon service callers use the final service boundary"
    ),
    migration_module!(
        "src/service/profile.rs",
        "service boundary exists while profile command call sites migrate incrementally",
        "remove once profile service callers use the final service boundary"
    ),
    migration_module!(
        "src/service/recording.rs",
        "service boundary exists while agent and CLI recording call sites migrate incrementally",
        "remove once recording service callers use the final service boundary"
    ),
    migration_module!(
        "src/service/report.rs",
        "service boundary exists while report command call sites migrate incrementally",
        "remove once report service callers use the final service boundary"
    ),
    migration_module!(
        "src/service/scenario.rs",
        "service boundary exists while scenario command call sites migrate incrementally",
        "remove once scenario service callers use the final service boundary"
    ),
    migration_module!(
        "src/session.rs",
        "session-stage context remains while tick extraction adopts shared context incrementally",
        "remove once tick extraction owns the context and no local dead code allowance is needed"
    ),
    migration_module!(
        "src/session/target.rs",
        "target-stage facade preserves paths while targeting.rs remains active",
        "remove once target-stage callers use the final module directly"
    ),
    migration_module!(
        "src/session/ticks/frame.rs",
        "frame tick context exists during session stage extraction",
        "remove once frame ticking is fully owned here and used by session runtime"
    ),
    migration_module!(
        "src/session/ticks/hardware.rs",
        "hardware tick context exists during session stage extraction",
        "remove once hardware ticking is fully owned here and used by session runtime"
    ),
    migration_module!(
        "src/session/ticks/mod.rs",
        "tick-context namespace exists during session stage extraction",
        "remove once tick contexts are fully owned by concrete tick modules"
    ),
    migration_module!(
        "src/session/ticks/probe.rs",
        "probe tick context exists during session stage extraction",
        "remove once probe ticking is fully owned here and used by session runtime"
    ),
    migration_module!(
        "src/session/ticks/summary.rs",
        "summary tick context exists during session stage extraction",
        "remove once summary ticking is fully owned here and used by session runtime"
    ),
    migration_module!(
        "src/session/ticks/target.rs",
        "target tick context exists during session stage extraction",
        "remove once target ticking is fully owned here and used by session runtime"
    ),
    migration_module!(
        "src/session/ticks/telemetry.rs",
        "telemetry tick context exists during session stage extraction",
        "remove once telemetry ticking is fully owned here and used by session runtime"
    ),
    migration_module!(
        "src/system/cgroup.rs",
        "system cgroup facade preserves low-level reader paths while root callers migrate",
        "remove once cgroup callers use the final system module directly"
    ),
    migration_module!(
        "src/system/command.rs",
        "system command-runner facade preserves low-level command paths while callers migrate",
        "remove once command callers use the final system module directly"
    ),
    migration_module!(
        "src/system/mod.rs",
        "system facade preserves low-level reader imports while root callers migrate",
        "remove once callers import concrete system modules directly"
    ),
    migration_module!(
        "src/system/sysfs.rs",
        "system sysfs facade preserves low-level reader paths while callers migrate",
        "remove once sysfs callers use the final system module directly"
    ),
];
