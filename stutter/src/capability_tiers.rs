//! User-facing capability boundary. A capability is `Supported` only when the live
//! evidence it names exists and verifies; `xtask capability-check` enforces that
//! against `docs/evidence/index.json`, so the tier cannot drift ahead of the evidence.
use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityTier {
    /// Contract, regression coverage and real-system evidence exist; documented limits.
    Supported,
    /// Usable but not yet backed by the evidence its target tier requires.
    Experimental,
    /// Implementation detail or test support; not a user surface.
    Internal,
    /// Present in the tree but refused at runtime.
    Disabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityClass {
    NonMutating,
    Mutating,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Capability {
    pub id: &'static str,
    pub title: &'static str,
    pub class: CapabilityClass,
    /// Tier justified by the evidence that exists today.
    pub tier: CapabilityTier,
    /// Tier intended for the first supported release.
    pub target_tier: CapabilityTier,
    pub commands: &'static [&'static str],
    /// Mutation provider kinds this capability governs (see `MUTATION_PROVIDER_KINDS`).
    pub providers: &'static [&'static str],
    /// Live-acceptance suites that must have a verified PASS bundle to be `Supported`
    /// (historical graduation proof, checked against `docs/evidence/index.json`).
    pub required_evidence: &'static [&'static str],
    /// Suites a release candidate must bring fresh, exact-commit bundles for (shipped-code
    /// proof). Graduation evidence that is too long to repeat per candidate, such as the
    /// coverage-qualified soak, is replaced here by its short counterpart.
    pub required_release_evidence: &'static [&'static str],
    pub limitations: &'static str,
}

/// Required for every mutating capability before it can be `Supported`.
pub const MUTATING_REQUIRED_EVIDENCE: [&str; 2] = ["live_recovery", "live_suspend_resume"];

const OBSERVATION: &[&str] = &["live_observation"];
const AFFINITY: &[&str] = &[
    "live_observation",
    "live_affinity",
    "live_recovery",
    "live_suspend_resume",
    "live_affinity_benefit",
];

const CGROUP_PLACEMENT: &[&str] = AFFINITY;

/// What a release candidate must prove for the graduated mutation capabilities: the real
/// observation path and the formal production-controller smoke, which itself drives both
/// graduated operations through the normal support registry.
const MUTATION_RELEASE: &[&str] = &["live_observation", "live_autotune_smoke"];
/// Historical graduation proof of the bounded controller: the recovery suites every mutating
/// capability needs, and the coverage-qualified production-controller soak.
const BOUNDED_CONTROLLER: &[&str] = &["live_recovery", "live_suspend_resume", "live_autotune_soak"];
/// The packaged bounded service graduated on targeted acceptance (observed, physically verified
/// behavior), not on elapsed time; the endurance suite `live_bounded_service` is a separate,
/// optional contract that this tier does not claim.
const BOUNDED_SERVICE: &[&str] = &[
    "live_recovery",
    "live_suspend_resume",
    "live_bounded_service_acceptance",
];

pub const CAPABILITIES: [Capability; 47] = [
    obs(
        "doctor",
        "Local preflight checks",
        &["doctor"],
        CapabilityTier::Supported,
        CapabilityTier::Supported,
    ),
    obs(
        "record",
        "Record scheduler and frame evidence",
        &["record"],
        CapabilityTier::Supported,
        CapabilityTier::Supported,
    ),
    obs(
        "bench",
        "Repeatable benchmark scenarios",
        &["bench", "scenario"],
        CapabilityTier::Supported,
        CapabilityTier::Supported,
    ),
    obs(
        "report",
        "Report and HTML rendering of recordings",
        &["report", "summary", "validate"],
        CapabilityTier::Supported,
        CapabilityTier::Supported,
    ),
    obs(
        "diagnose",
        "Evidence-based stutter diagnosis",
        &["report", "check"],
        CapabilityTier::Experimental,
        CapabilityTier::Experimental,
    ),
    obs(
        "advisor",
        "Advisor and fix-plan recommendations",
        &["advisor", "recommend"],
        CapabilityTier::Experimental,
        CapabilityTier::Experimental,
    ),
    obs(
        "tune_evaluation",
        "Baseline/candidate evaluation of tunings",
        &["tune", "recommend", "prove-fix"],
        CapabilityTier::Experimental,
        CapabilityTier::Experimental,
    ),
    obs(
        "audit",
        "Inspect local audit events",
        &["audit"],
        CapabilityTier::Supported,
        CapabilityTier::Supported,
    ),
    Capability {
        id: "restore",
        title: "Restore recorded CPU-affinity state",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["restore"],
        providers: &[],
        required_evidence: &["live_affinity", "live_recovery", "live_suspend_resume"],
        required_release_evidence: &["live_affinity", "live_recovery", "live_suspend_resume"],
        limitations: "Restores only the exact process/task identities recorded; replacement identities are skipped.",
    },
    Capability {
        id: "cpu_affinity_experiment",
        title: "Autonomous CPU-affinity tuning of a game under scheduler pressure",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Supported,
        target_tier: CapabilityTier::Supported,
        commands: &["tune", "autotune"],
        providers: &["cpu_affinity_profile"],
        required_evidence: AFFINITY,
        required_release_evidence: MUTATION_RELEASE,
        limitations: "Supported autonomous apply only for a stable, explicitly identified game under scheduler pressure, judged by its own runnable latency (GameRunnableLatency); every other workload, pressure or objective stays suggest-only or manual. Low-risk, per-process-tree scope.",
    },
    Capability {
        id: "uclamp_experiment",
        title: "Per-task utilisation clamps (uclamp)",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["autotune"],
        providers: &["uclamp"],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "Medium-risk; evaluated by its own graduation campaign and does not inherit CPU-affinity maturity.",
    },
    Capability {
        id: "nice_experiment",
        title: "Process niceness (nice)",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["autotune"],
        providers: &["nice"],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "Medium-risk; evaluated by its own graduation campaign and does not inherit CPU-affinity maturity.",
    },
    Capability {
        id: "ioprio_experiment",
        title: "I/O priority (ionice)",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["autotune"],
        providers: &["ionice"],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "Medium-risk; evaluated by its own graduation campaign and does not inherit CPU-affinity maturity.",
    },
    Capability {
        id: "cgroup_placement_experiment",
        title: "Autonomous cgroup placement of a game under scheduler pressure",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Supported,
        target_tier: CapabilityTier::Supported,
        commands: &["autotune"],
        providers: &["cgroup_placement"],
        required_evidence: CGROUP_PLACEMENT,
        required_release_evidence: MUTATION_RELEASE,
        limitations: "Supported autonomous apply only for a stable, explicitly identified game under scheduler pressure, moved into the administrator-configured game cgroup, judged by its own runnable latency (GameRunnableLatency); every other workload, pressure or objective stays suggest-only or manual. Medium-risk; evaluated by its own graduation campaign and does not inherit CPU-affinity maturity.",
    },
    Capability {
        id: "cpu_power_epp",
        title: "CPU energy-performance preference (EPP)",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["autotune"],
        providers: &["cpu_power"],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "System-wide effect. Reversible per-policy sysfs write; first system-wide candidate for autonomy. Does not inherit another operation's maturity.",
    },
    Capability {
        id: "cpu_power_governor",
        title: "CPU scaling governor",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["autotune"],
        providers: &["cpu_power"],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "System-wide effect. Governor mutation is manual / high-risk and never autonomous. Does not inherit another operation's maturity.",
    },
    Capability {
        id: "gpu_power_profile",
        title: "GPU power profile mode",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["autotune"],
        providers: &["gpu_power"],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "System-wide effect. Profile-only (pp_power_profile_mode); no forced performance level. Does not inherit another operation's maturity.",
    },
    Capability {
        id: "gpu_force_performance",
        title: "GPU forced performance level (DPM)",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["autotune"],
        providers: &["gpu_power"],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "System-wide effect. Forced DPM is manual / high-risk and never autonomous. Does not inherit another operation's maturity.",
    },
    Capability {
        id: "vm_swappiness",
        title: "vm.swappiness",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["autotune"],
        providers: &["vm_knob"],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "System-wide effect. Boot-aware recovery required; evaluated by its own campaign. Does not inherit another operation's maturity.",
    },
    Capability {
        id: "vm_dirty_background_ratio",
        title: "vm.dirty_background_ratio",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["autotune"],
        providers: &["vm_knob"],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "System-wide effect. Dirty-ratio knobs are manual / high-risk and never autonomous. Does not inherit another operation's maturity.",
    },
    Capability {
        id: "vm_dirty_ratio",
        title: "vm.dirty_ratio",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["autotune"],
        providers: &["vm_knob"],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "System-wide effect. Dirty-ratio knobs are manual / high-risk and never autonomous. Does not inherit another operation's maturity.",
    },
    Capability {
        id: "irq_affinity_known_device",
        title: "IRQ affinity of a known device",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["autotune"],
        providers: &["irq_affinity"],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "System-wide effect. Needs a stable device identity and its own target-impact campaign. Does not inherit another operation's maturity.",
    },
    Capability {
        id: "irq_affinity_storage_or_unknown",
        title: "IRQ affinity of storage or unknown devices",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["autotune"],
        providers: &["irq_affinity"],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "System-wide effect. Storage and unknown IRQs are manual / high-risk and never autonomous. Does not inherit another operation's maturity.",
    },
    Capability {
        id: "pm_qos_cpu_latency",
        title: "PM QoS CPU wake-latency lease",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["autotune"],
        providers: &["pm_qos"],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "System-wide effect: keeps every CPU out of deep idle states while held. A worker-owned descriptor lease with an enforced hold deadline; manual only, never autonomous, not a low-risk default. Does not inherit another operation's maturity.",
    },
    Capability {
        id: "resctrl_cache_allocation",
        title: "resctrl L3 cache allocation group",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["autotune"],
        providers: &["resctrl"],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "Shared-cache allocation affects every task on the host. Owned group with a validated capacity bitmask and a default-group bystander check; manual only, never autonomous, no task assignment. Does not inherit another operation's maturity.",
    },
    Capability {
        id: "damos_scheme",
        title: "DAMOS scheme on an idle kdamond",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["autotune"],
        providers: &["damos"],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "Reclaim and LRU actions are irreversible in effect; recovery is manual and never claims pages were restored. Idle kdamond only, verified target, bounded quota; manual only, never autonomous. Does not inherit another operation's maturity.",
    },
    Capability {
        id: "cgroup_cpu_weight",
        title: "Cgroup cpu.weight write",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["autotune"],
        providers: &["cgroup_control"],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "Delegated group only, preflighted hierarchy, foreign changes abort, restore only while the value is still ours; manual only, never autonomous. Does not inherit cgroup placement or another knob's maturity.",
    },
    Capability {
        id: "cgroup_cpu_idle",
        title: "Cgroup cpu.idle write",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["autotune"],
        providers: &["cgroup_control"],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "Delegated group only, preflighted hierarchy, foreign changes abort, restore only while the value is still ours; manual only, never autonomous. Does not inherit cgroup placement or another knob's maturity.",
    },
    Capability {
        id: "cgroup_cpu_max",
        title: "Cgroup cpu.max write",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["autotune"],
        providers: &["cgroup_control"],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "Delegated group only, preflighted hierarchy, foreign changes abort, restore only while the value is still ours; manual only, never autonomous. Does not inherit cgroup placement or another knob's maturity.",
    },
    Capability {
        id: "cgroup_cpuset_cpus",
        title: "Cgroup cpuset.cpus write",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["autotune"],
        providers: &["cgroup_control"],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "Delegated group only, preflighted hierarchy, foreign changes abort, restore only while the value is still ours; manual only, never autonomous. Does not inherit cgroup placement or another knob's maturity.",
    },
    Capability {
        id: "cgroup_cpuset_mems",
        title: "Cgroup cpuset.mems write",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["autotune"],
        providers: &["cgroup_control"],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "Delegated group only, preflighted hierarchy, foreign changes abort, restore only while the value is still ours; manual only, never autonomous. Does not inherit cgroup placement or another knob's maturity.",
    },
    Capability {
        id: "cgroup_memory_min",
        title: "Cgroup memory.min write",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["autotune"],
        providers: &["cgroup_control"],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "Delegated group only, preflighted hierarchy, foreign changes abort, restore only while the value is still ours; manual only, never autonomous. Does not inherit cgroup placement or another knob's maturity.",
    },
    Capability {
        id: "cgroup_memory_low",
        title: "Cgroup memory.low write",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["autotune"],
        providers: &["cgroup_control"],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "Delegated group only, preflighted hierarchy, foreign changes abort, restore only while the value is still ours; manual only, never autonomous. Does not inherit cgroup placement or another knob's maturity.",
    },
    Capability {
        id: "cgroup_memory_high",
        title: "Cgroup memory.high write",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["autotune"],
        providers: &["cgroup_control"],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "Delegated group only, preflighted hierarchy, foreign changes abort, restore only while the value is still ours; manual only, never autonomous. Does not inherit cgroup placement or another knob's maturity.",
    },
    Capability {
        id: "cgroup_io_weight",
        title: "Cgroup io.weight write",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["autotune"],
        providers: &["cgroup_control"],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "Delegated group only, preflighted hierarchy, foreign changes abort, restore only while the value is still ours; manual only, never autonomous. Does not inherit cgroup placement or another knob's maturity.",
    },
    Capability {
        id: "block_io_scheduler",
        title: "Block I/O scheduler selection",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["autotune"],
        providers: &["block_scheduler"],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "Device-wide effect. Authorized, unprotected device only; the scheduler must be one the device lists; restore only on the same device while ours is active. Manual only, never autonomous. Does not inherit another operation's maturity.",
    },
    Capability {
        id: "sched_ext_scheduler_strategy",
        title: "sched_ext scheduler strategy switch",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["autotune"],
        providers: &["sched_ext_strategy"],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "Global effect on every task. Allowlisted scheduler and reviewed arguments only, exact restore of the prior scheduler and mode, offline campaign authorization; manual only, never autonomous, no second SCX daemon. Does not inherit another operation's maturity.",
    },
    Capability {
        id: "numa_migrate_pages",
        title: "NUMA page migration",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["autotune"],
        providers: &["numa_migration"],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "Irreversible in effect: moved pages are not returned and recovery is manual. Verified target by start time, online disjoint nodes, acknowledgement required; manual only, never autonomous. Moving CPU affinity is not moving memory. Does not inherit another operation's maturity.",
    },
    Capability {
        id: "net_tcp_congestion_control",
        title: "TCP congestion control",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["autotune"],
        providers: &["net_sysctl"],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "Namespace-wide state. A bounded value domain, bound to its network namespace, refused while bpftune owns the setting (UnsupportedDelegation); manual only, never autonomous, no live public traffic test. Does not inherit another operation's maturity.",
    },
    Capability {
        id: "net_tcp_buffers",
        title: "TCP receive and send buffers",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["autotune"],
        providers: &["net_sysctl"],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "Namespace-wide state. A bounded value domain, bound to its network namespace, refused while bpftune owns the setting (UnsupportedDelegation); manual only, never autonomous, no live public traffic test. Does not inherit another operation's maturity.",
    },
    Capability {
        id: "net_busy_poll",
        title: "Network busy-poll interval",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["autotune"],
        providers: &["net_sysctl"],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "Namespace-wide state. A bounded value domain, bound to its network namespace, refused while bpftune owns the setting (UnsupportedDelegation); manual only, never autonomous, no live public traffic test. Does not inherit another operation's maturity.",
    },
    Capability {
        id: "bounded_autotune_controller",
        title: "Bounded autonomous autotune controller",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Supported,
        target_tier: CapabilityTier::Supported,
        commands: &["autotune", "autotune-status"],
        providers: &[],
        required_evidence: BOUNDED_CONTROLLER,
        required_release_evidence: &["live_autotune_smoke"],
        limitations: "Autonomous mutation is limited to the exact entries of the autonomous support registry (AUTONOMOUS_CONTRACTS): CPU-affinity profile and cgroup placement, each for a stable, explicitly identified game under scheduler pressure judged by GameRunnableLatency (affinity: local process tree; cgroup: administrator-configured game cgroup). It authorises no third operation, no system-wide operation, and not the daemon/service integration (always_on_autotune).",
    },
    Capability {
        id: "always_on_autotune",
        title: "Always-on autotune daemon and service",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Supported,
        target_tier: CapabilityTier::Supported,
        commands: &["daemon", "service", "autotune-status"],
        providers: &[],
        required_evidence: BOUNDED_SERVICE,
        required_release_evidence: &["live_autotune_smoke", "live_packaged_service_smoke"],
        limitations: "Supported only as the packaged bounded systemd service: a controller running as the monitored desktop account plus a separate root worker (the units under packaging/systemd, installed with `stutter service install --mode system-bounded-autotune` or an equivalent that runs the same units from an immutable root-owned binary), heuristic auto-focus at the qualified confidence floor, apply-medium-risk, and only the two entries of AUTONOMOUS_CONTRACTS (CPU-affinity profile and cgroup placement for a stable game under scheduler pressure). Qualified by targeted acceptance (live_bounded_service_acceptance) and a production boot check on one NixOS host. Not claimed: extended endurance (eight hours, six cycles, in-campaign reboots; live_bounded_service), OpenRC, any other daemon mode, any system-wide or high-risk operation, remote agent mutation; a release candidate must bring a fresh exact-build `live_packaged_service_smoke` of the installed units.",
    },
    Capability {
        id: "remote_agent_mutation",
        title: "Remote agent endpoints",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["agent"],
        providers: &[],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "Remote mutation remains experimental.",
    },
    Capability {
        id: "privileged_worker",
        title: "Privileged worker used by service paths",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Internal,
        target_tier: CapabilityTier::Internal,
        commands: &["privileged-worker"],
        providers: &[],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "Implementation detail of the service path; not a user surface.",
    },
    Capability {
        id: "fake_action_provider",
        title: "Synthetic test action provider",
        class: CapabilityClass::NonMutating,
        tier: CapabilityTier::Internal,
        target_tier: CapabilityTier::Internal,
        commands: &[],
        providers: &[],
        required_evidence: &[],
        required_release_evidence: &[],
        limitations: "Test support only.",
    },
    obs(
        "completions_and_man",
        "Shell completions and manual page",
        &["completions", "man"],
        CapabilityTier::Supported,
        CapabilityTier::Supported,
    ),
];

const fn obs(
    id: &'static str,
    title: &'static str,
    commands: &'static [&'static str],
    tier: CapabilityTier,
    target_tier: CapabilityTier,
) -> Capability {
    Capability {
        id,
        title,
        class: CapabilityClass::NonMutating,
        tier,
        target_tier,
        commands,
        providers: &[],
        required_evidence: OBSERVATION,
        required_release_evidence: OBSERVATION,
        limitations: match tier {
            CapabilityTier::Supported => {
                "Observation is useful without mutation; backed by indexed real-system observation, kernel and overhead evidence."
            }
            _ => {
                "Observation is useful without mutation; real-system evidence is pending (see docs/evidence/index.json)."
            }
        },
    }
}

pub fn capability_registry() -> &'static [Capability] {
    &CAPABILITIES
}

/// Human-readable listing for `stutter capabilities`.
pub fn render_capabilities_text() -> String {
    let mut text =
        String::from("Capability tiers (Supported / Experimental / Internal / Disabled)\n");
    for capability in capability_registry() {
        text.push_str(&format!(
            "- {} [{:?}, target {:?}] {}\n    class: {:?}; commands: {}\n    required live evidence: {}\n    required release evidence: {}\n    {}\n",
            capability.id,
            capability.tier,
            capability.target_tier,
            capability.title,
            capability.class,
            if capability.commands.is_empty() { "-".to_owned() } else { capability.commands.join(", ") },
            if capability.required_evidence.is_empty() { "-".to_owned() } else { capability.required_evidence.join(", ") },
            if capability.required_release_evidence.is_empty() { "-".to_owned() } else { capability.required_release_evidence.join(", ") },
            capability.limitations,
        ));
    }
    text.push_str("A capability is Supported only when its required live evidence is verified.\n");
    text
}

#[cfg(test)]
#[path = "capability_tiers_tests.rs"]
mod tests;
