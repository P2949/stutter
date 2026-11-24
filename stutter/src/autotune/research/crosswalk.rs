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

pub(super) const SPECS: [Spec; 34] = [
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
    /// What the plan intends this dossier to be for Cyberpunk. It is a role, not a result.
    #[serde(rename = "planned_closure")]
    pub closure: Closure,
    /// `ObservedInCampaign` only when the campaign supplied a digest of its own evidence for this row;
    /// otherwise `NotObservedInThisCampaign`, whatever the plan intended.
    pub observed_disposition: &'static str,
    /// Why the row has the disposition it has: the evidence it rests on, or the typed reason there is none (no resident
    /// producer feeds it, the produced signal was below its floor, or the plan itself blocks or excludes it). Absence of
    /// evidence is never left to read as a proven exclusion, and an exclusion is never left to read as a missing signal.
    pub observed_reason: &'static str,
    pub observed_source_sha256: Option<String>,
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

/// The disposition of a row in a campaign and why. A row the plan blocks or excludes says so; a row the plan wants observed
/// is observed only with the campaign's own evidence, and otherwise says whether a resident producer exists for it at all.
fn disposition(id: &str, planned: Closure, observed: bool) -> (&'static str, &'static str) {
    if observed {
        return ("ObservedInCampaign", "EvidenceVerified");
    }
    match planned {
        Closure::BlockedHardware => ("BlockedHardware", "PlannedBlockedByHardware"),
        Closure::BlockedPolicy => ("BlockedPolicy", "PlannedBlockedByPolicy"),
        Closure::ExcludedGameApi => ("ExcludedGameApi", "PlannedExclusionTheGameCannotUse"),
        Closure::AlreadySatisfied | Closure::FutureGraduation => {
            ("NotAnObservationRole", "PlannedAsNeitherObservedNorTested")
        }
        Closure::Tested => (
            "NotObservedInThisCampaign",
            "PlannedAsATestNotAnObservation",
        ),
        // A resident producer exists for the three signals the runtime feeds; for them the campaign's signal was absent
        // or below its floor. For every other dossier nothing in the resident runtime feeds it yet.
        Closure::Observed if super::crosswalk_producer::produces(id) => (
            "NotObservedInThisCampaign",
            "ProducedSignalAbsentOrBelowItsFloor",
        ),
        Closure::Observed => (
            "NotObservedInThisCampaign",
            "NoResidentProducerFeedsThisDossier",
        ),
    }
}

/// Joins the static Cyberpunk roles to the integration coverage ledger. Exactly 34 rows are
/// required; a ledger with an unknown or missing id is rejected rather than silently padded.
pub fn build_rows(coverage_json: &str) -> Result<Vec<Row>, CrosswalkError> {
    build_rows_observed(coverage_json, &std::collections::BTreeMap::new())
}

/// As `build_rows`, with the digest of the campaign's own evidence for each row it actually observed. A row
/// with no entry is not claimed as observed, so a planned role never reads as a result.
pub fn build_rows_observed(
    coverage_json: &str,
    observed: &std::collections::BTreeMap<String, String>,
) -> Result<Vec<Row>, CrosswalkError> {
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
            observed_disposition: disposition(spec.0, spec.1, observed.contains_key(spec.0)).0,
            observed_reason: disposition(spec.0, spec.1, observed.contains_key(spec.0)).1,
            observed_source_sha256: observed.get(spec.0).cloned(),
            evidence_digest: digest,
            next_bug_if_any: None,
        });
    }
    Ok(rows)
}

/// The file, beside the study, in which the resident runtime records which dossiers a campaign actually observed,
/// each with the SHA-256 of the evidence it rests on.
pub const OBSERVATIONS_FILE: &str = "crosswalk-observations.json";

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
    fn a_planned_role_is_never_an_observation_unless_the_campaign_supplies_its_evidence() {
        let plain = build_rows(&ledger()).expect("rows");
        assert!(
            plain
                .iter()
                .all(|r| r.observed_disposition != "ObservedInCampaign"
                    && r.observed_source_sha256.is_none())
        );
        let observed = [("5.1".to_owned(), "ab".repeat(32))].into();
        let rows = build_rows_observed(&ledger(), &observed).expect("rows");
        assert_eq!(
            rows.iter()
                .filter(|r| r.observed_disposition == "ObservedInCampaign")
                .count(),
            1
        );
        let one = rows.iter().find(|r| r.id == "5.1").unwrap();
        assert_eq!(
            one.observed_source_sha256.as_deref(),
            Some(&"ab".repeat(32)[..])
        );
        let json = serde_json::to_value(one).unwrap();
        assert!(json.get("planned_closure").is_some() && json.get("closure").is_none());
    }

    /// Every one of the 34 rows says why it is not observed: a row the plan blocks or excludes says so (never a missing
    /// signal), one the plan wants observed says whether a resident producer exists for it, and none is left unexplained.
    #[test]
    fn every_row_without_evidence_has_a_typed_reason_and_a_block_is_never_a_missing_signal() {
        let rows = build_rows(&ledger()).expect("rows");
        assert!(rows.iter().all(|r| !r.observed_reason.is_empty()));
        for row in &rows {
            match row.closure {
                Closure::BlockedHardware => assert_eq!(
                    (row.observed_disposition, row.observed_reason),
                    ("BlockedHardware", "PlannedBlockedByHardware"),
                    "{}",
                    row.id
                ),
                Closure::BlockedPolicy => assert_eq!(
                    (row.observed_disposition, row.observed_reason),
                    ("BlockedPolicy", "PlannedBlockedByPolicy"),
                    "{}",
                    row.id
                ),
                Closure::ExcludedGameApi => {
                    assert_eq!(row.observed_disposition, "ExcludedGameApi", "{}", row.id)
                }
                Closure::Tested => assert_eq!(
                    row.observed_reason, "PlannedAsATestNotAnObservation",
                    "{}",
                    row.id
                ),
                Closure::Observed => assert_eq!(
                    row.observed_reason,
                    if matches!(
                        row.id.as_str(),
                        "5.9" | "5.13" | "5.17" | "5.19" | "5.20" | "5.22"
                    ) {
                        "ProducedSignalAbsentOrBelowItsFloor"
                    } else {
                        "NoResidentProducerFeedsThisDossier"
                    },
                    "{}",
                    row.id
                ),
                Closure::AlreadySatisfied | Closure::FutureGraduation => {
                    assert_eq!(
                        row.observed_disposition, "NotAnObservationRole",
                        "{}",
                        row.id
                    )
                }
            }
        }
        // With evidence, the reason is the evidence.
        let observed = [("5.13".to_owned(), "ab".repeat(32))].into();
        let with = build_rows_observed(&ledger(), &observed).expect("rows");
        let row = with.iter().find(|r| r.id == "5.13").unwrap();
        assert_eq!(
            (row.observed_disposition, row.observed_reason),
            ("ObservedInCampaign", "EvidenceVerified")
        );
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

    /// Write the 34-row disposition artifact: `STUTTER_CROSSWALK_OUT=<file> cargo test
    /// write_crosswalk_artifact -- --ignored`. It reads the ledger and writes only that file.
    #[test]
    #[ignore = "writes a private artifact to STUTTER_CROSSWALK_OUT"]
    fn write_crosswalk_artifact() {
        let out = std::env::var_os("STUTTER_CROSSWALK_OUT").expect("set STUTTER_CROSSWALK_OUT");
        let rows = build_rows(&ledger()).expect("rows");
        let mut by_closure = std::collections::BTreeMap::<String, u32>::new();
        for row in &rows {
            *by_closure.entry(format!("{:?}", row.closure)).or_default() += 1;
        }
        let document = serde_json::json!({
            "rows": rows,
            "planned_closure_counts": by_closure,
            "note": "A row closing as Observed/Tested/Excluded/Blocked/AlreadySatisfied/FutureGraduation is not a game-benefit validation; none is GameBenefitValidated."
        });
        std::fs::write(out, serde_json::to_vec_pretty(&document).unwrap()).unwrap();
    }
}
