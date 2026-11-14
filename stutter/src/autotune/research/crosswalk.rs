//! Machine-readable Cyberpunk role for each of the 34 integration dossiers (PLAN section 5).
//! A row closes as Observed/Tested/Excluded/Blocked/AlreadySatisfied/FutureGraduation; none
//! becomes `GameBenefitValidated` because its source fixture passes.

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Closure {
    AlreadySatisfied,
    Observed,
    Tested,
    BlockedHardware,
    BlockedPolicy,
    ExcludedGameApi,
    FutureGraduation,
}

/// (id, closure, permission, cyberpunk role, specific test or exclusion)
type Spec = (
    &'static str,
    Closure,
    &'static str,
    &'static str,
    &'static str,
);

const SPECS: [Spec; 34] = [
    (
        "5.1",
        Closure::Observed,
        "observe",
        "owner: live ClientCount and game PID",
        "read D-Bus state during every attempt; no actuation",
    ),
    (
        "5.2",
        Closure::Observed,
        "observe",
        "owner: active task-priority changes",
        "installed inert tool is not running; no rules enabled",
    ),
    (
        "5.3",
        Closure::Observed,
        "observe",
        "owner: service and readable properties",
        "setter-only interface is UnsupportedInterface",
    ),
    (
        "5.4",
        Closure::Observed,
        "observe",
        "owner: active profile drift",
        "compare with baseline; no profile toggle",
    ),
    (
        "5.5",
        Closure::Observed,
        "observe",
        "owner: active eBPF resource domain",
        "installed executable alone is not an owner",
    ),
    (
        "5.6",
        Closure::Observed,
        "observe",
        "actual scheduler and weight semantics",
        "context only; no global scheduler switch",
    ),
    (
        "5.7",
        Closure::BlockedHardware,
        "observe",
        "owner: real system-bus loader",
        "absent on this host; kernel-thread false positive stays fixed",
    ),
    (
        "5.8",
        Closure::BlockedPolicy,
        "observe",
        "module-owned kdamond diagnostic",
        "DAMOS reclaim is forbidden by plan",
    ),
    (
        "5.9",
        Closure::Observed,
        "observe",
        "six physical cores, SMT siblings, cpuset",
        "topology fingerprint per block; no assumed isolated cores",
    ),
    (
        "5.10",
        Closure::Tested,
        "observe",
        "selective cycles/instructions/IPC on render thread",
        "invalid time_enabled/time_running becomes unknown",
    ),
    (
        "5.11",
        Closure::Tested,
        "observe",
        "runnable vs CPU/IRQ/IO ambiguity",
        "selective, bounded overhead, no privilege expansion",
    ),
    (
        "5.12",
        Closure::Tested,
        "observe",
        "one authenticated frame producer plus compositor/DRM",
        "frame-primary keep stays ineligible without the 2 ms gate",
    ),
    (
        "5.13",
        Closure::BlockedPolicy,
        "observe",
        "RX 9070 XT clocks/temperature/power/reset",
        "GPU electrical changes quarantined after the prior reset",
    ),
    (
        "5.14",
        Closure::AlreadySatisfied,
        "none",
        "legal configuration search reuse",
        "algorithm reuse inside the frozen budget; no external install",
    ),
    (
        "5.15",
        Closure::AlreadySatisfied,
        "none",
        "rank legal Proton candidates with multiplicity",
        "no compile-only utility transfer",
    ),
    (
        "5.16",
        Closure::Observed,
        "observe",
        "owner: exact power-policy state",
        "binaries on PATH are inert; no global switch",
    ),
    (
        "5.17",
        Closure::Tested,
        "observe",
        "guardrail: CPU frequency/EPP/thermal drift",
        "drift splits cohort; no governor change",
    ),
    (
        "5.18",
        Closure::Observed,
        "observe",
        "IRQ placement drift vs renderer/IO events",
        "no live IRQ reassignment",
    ),
    (
        "5.19",
        Closure::ExcludedGameApi,
        "observe",
        "one online NUMA node",
        "no inter-node migration is possible",
    ),
    (
        "5.20",
        Closure::Tested,
        "observe",
        "guardrail: memory PSI, swap, OOM kills",
        "OOM protection is never disabled",
    ),
    (
        "5.21",
        Closure::Tested,
        "supported-action",
        "game cgroup membership and configured target",
        "CgroupPlacement only into an administrator-configured cgroup; OpenRC excluded",
    ),
    (
        "5.22",
        Closure::Observed,
        "observe",
        "system and game-cgroup CPU/memory/IO stalls",
        "no inference from unrelated pressure alone",
    ),
    (
        "5.23",
        Closure::BlockedPolicy,
        "observe",
        "/dev/cpu_dma_latency owner",
        "manual/high-risk; no lease during Cyberpunk",
    ),
    (
        "5.24",
        Closure::BlockedHardware,
        "none",
        "resctrl/Intel RDT",
        "i5-10600K shows no RDT allocation; typed unavailable",
    ),
    (
        "5.25",
        Closure::FutureGraduation,
        "none",
        "Stutter daemon overhead only",
        "only if measured overhead is meaningful; game is never recompiled",
    ),
    (
        "5.26",
        Closure::Observed,
        "observe",
        "shader/asset IO and background pressure",
        "no root NVMe scheduler or global IO priority change",
    ),
    (
        "5.27",
        Closure::ExcludedGameApi,
        "none",
        "Steam traffic as confounder",
        "single-player benchmark has no network objective; no sysctls",
    ),
    (
        "5.28",
        Closure::Tested,
        "none",
        "bounded owned pressure and observer overhead cross-check",
        "only resident product actions count as autonomous",
    ),
    (
        "5.29",
        Closure::Tested,
        "none",
        "external comparator for owned fixtures",
        "product action path is proven separately; never filled by taskset",
    ),
    (
        "extra.mpi",
        Closure::ExcludedGameApi,
        "none",
        "algorithm analogy only",
        "no MPI on Cyberpunk",
    ),
    (
        "extra.diag",
        Closure::Tested,
        "observe",
        "confirm pidstat/iostat/numastat on real ambiguity",
        "bounded overhead",
    ),
    (
        "extra.kernels",
        Closure::BlockedPolicy,
        "none",
        "record active kernel only",
        "no alternate kernel campaign; no automatic reboot",
    ),
    (
        "extra.boot",
        Closure::Observed,
        "observe",
        "nohz_full/isolcpus/rcu_nocbs/housekeeping",
        "boot flags are never edited to chase FPS",
    ),
    (
        "extra.workloads",
        Closure::Tested,
        "observe",
        "foreground guardrail and owned competition",
        "background services are never sacrificed",
    ),
];

#[derive(Debug, Serialize)]
pub struct Row {
    pub id: String,
    pub code_anchor: String,
    pub current_grade: String,
    pub source_provenance: String,
    pub host_availability: String,
    pub resident_access: &'static str,
    pub cyberpunk_role: &'static str,
    pub specific_test_or_exclusion: &'static str,
    pub signal_quality: &'static str,
    pub permission: &'static str,
    pub closure: Closure,
    pub evidence_digest: String,
    pub next_bug_if_any: Option<&'static str>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CrosswalkError {
    Malformed,
    CountMismatch(usize),
    UnknownId(String),
    MissingId(&'static str),
}

/// Joins the static Cyberpunk roles to the integration coverage ledger. Exactly 34 rows are
/// required; a ledger with an unknown or missing id is rejected rather than silently padded.
pub fn build_rows(coverage_json: &str) -> Result<Vec<Row>, CrosswalkError> {
    let doc: serde_json::Value =
        serde_json::from_str(coverage_json).map_err(|_| CrosswalkError::Malformed)?;
    let entries = doc
        .get("entries")
        .and_then(|e| e.as_array())
        .ok_or(CrosswalkError::Malformed)?;
    if entries.len() != SPECS.len() {
        return Err(CrosswalkError::CountMismatch(entries.len()));
    }
    let source_sha = doc.get("source_sha").and_then(|s| s.as_str()).unwrap_or("");
    let text = |v: &serde_json::Value, k: &str| {
        v.get(k)
            .and_then(|s| s.as_str())
            .unwrap_or("unknown")
            .to_owned()
    };
    for entry in entries {
        let id = text(entry, "id");
        if !SPECS.iter().any(|s| s.0 == id) {
            return Err(CrosswalkError::UnknownId(id));
        }
    }
    let mut rows = Vec::with_capacity(SPECS.len());
    for spec in SPECS {
        let entry = entries
            .iter()
            .find(|e| e.get("id").and_then(|s| s.as_str()) == Some(spec.0))
            .ok_or(CrosswalkError::MissingId(spec.0))?;
        let digest = stutter_core::hash::sha256_hex(
            &serde_json::to_vec(entry).map_err(|_| CrosswalkError::Malformed)?,
        );
        let anchor = entry
            .get("anchors")
            .and_then(|a| a.get(0))
            .and_then(|a| a.as_str())
            .unwrap_or("unknown");
        rows.push(Row {
            id: spec.0.to_owned(),
            code_anchor: anchor.to_owned(),
            current_grade: text(entry, "status"),
            source_provenance: format!("integration-coverage.json@{source_sha}"),
            host_availability: text(entry, "runtime_availability"),
            resident_access: if spec.2 == "none" {
                "none"
            } else {
                "resident-read"
            },
            cyberpunk_role: spec.3,
            specific_test_or_exclusion: spec.4,
            signal_quality: "per-attempt provider quality; absent is unknown, never zero",
            permission: spec.2,
            closure: spec.1,
            evidence_digest: digest,
            next_bug_if_any: None,
        });
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ledger() -> String {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../docs/integration-coverage.json"
        );
        std::fs::read_to_string(path).expect("coverage ledger")
    }

    #[test]
    fn emits_exactly_thirty_four_rows_with_digests() {
        let rows = build_rows(&ledger()).expect("rows");
        assert_eq!(rows.len(), 34);
        assert!(rows.iter().all(|r| r.evidence_digest.len() == 64));
        assert!(rows.iter().all(|r| r.current_grade == "code_complete"));
    }

    #[test]
    fn only_cgroup_dossier_may_name_a_supported_action() {
        let rows = build_rows(&ledger()).expect("rows");
        let supported: Vec<_> = rows
            .iter()
            .filter(|r| r.permission == "supported-action")
            .collect();
        assert_eq!(supported.len(), 1);
        assert_eq!(supported[0].id, "5.21");
    }

    #[test]
    fn rejects_thirty_three_and_unknown_ids() {
        let mut doc: serde_json::Value = serde_json::from_str(&ledger()).expect("json");
        doc["entries"].as_array_mut().expect("entries").pop();
        assert!(matches!(
            build_rows(&doc.to_string()),
            Err(CrosswalkError::CountMismatch(33))
        ));
        let mut doc: serde_json::Value = serde_json::from_str(&ledger()).expect("json");
        doc["entries"][0]["id"] = "9.9".into();
        assert!(matches!(
            build_rows(&doc.to_string()),
            Err(CrosswalkError::UnknownId(_))
        ));
    }
}
