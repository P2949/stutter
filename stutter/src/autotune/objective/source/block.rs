//! Immutable raw blocks made by the authenticated collector, not decoded from client evidence.
use std::collections::BTreeMap;

use serde::Serialize;

use super::{KpiContract, Sample, SampleStore, Semantics, SourceBinding, WindowAggregate};

#[derive(Clone, Debug)]
pub struct KpiBlockRequest {
    pub block_id: String,
    pub run_id: String,
    pub work_unit: String,
    pub context_fingerprint: String,
    pub from_ns: u64,
    pub to_ns: u64,
}

/// Constructed only from samples accepted by a live, exact-process collector. Deliberately lacks
/// Deserialize: imported JSON or a matching hash cannot manufacture collector provenance.
#[derive(Clone, Debug, Serialize)]
pub struct ObservedKpiBlock {
    schema_version: u32,
    block_id: String,
    run_id: String,
    work_unit: String,
    context_fingerprint: String,
    source_session: String,
    binding: SourceBinding,
    boot_id: String,
    clock_namespace: crate::telemetry::proc_view::ClockNamespaceIdentity,
    executable_identity: (u64, u64),
    contract_digest: String,
    from_ns: u64,
    to_ns: u64,
    samples: Vec<Sample>,
}

impl ObservedKpiBlock {
    pub(super) fn capture(
        request: KpiBlockRequest,
        source_session: String,
        boot_id: String,
        clock_namespace: crate::telemetry::proc_view::ClockNamespaceIdentity,
        executable_identity: (u64, u64),
        contract: &KpiContract,
        store: &SampleStore,
    ) -> anyhow::Result<Self> {
        for value in [
            &request.block_id,
            &request.run_id,
            &request.work_unit,
            &request.context_fingerprint,
        ] {
            anyhow::ensure!(
                !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control),
                "block identity must be nonempty, bounded and printable"
            );
        }
        anyhow::ensure!(
            request.from_ns <= request.to_ns,
            "block interval is reversed"
        );
        let samples = store.block_samples(request.from_ns, request.to_ns);
        anyhow::ensure!(
            !samples.is_empty() && samples.len() <= 8192,
            "raw block is empty or exceeds 8192 samples"
        );
        anyhow::ensure!(
            samples
                .iter()
                .all(|sample| sample.work_unit.as_deref() == Some(&request.work_unit)),
            "block mixes or omits completed-work identities"
        );
        // Capacity loss must be checked against the original store before exporting its samples.
        for spec in std::iter::once(&contract.primary).chain(&contract.hard_guardrails) {
            let aggregate = store
                .window(&spec.id, request.from_ns, request.to_ns, request.to_ns)
                .ok_or_else(|| anyhow::anyhow!("missing or stale required KPI {}", spec.id))?;
            anyhow::ensure!(
                aggregate.complete && aggregate.semantics == Semantics::Direct,
                "incomplete or diagnostic-only required KPI {}",
                spec.id
            );
        }
        let block = Self {
            schema_version: 1,
            block_id: request.block_id,
            run_id: request.run_id,
            work_unit: request.work_unit,
            context_fingerprint: request.context_fingerprint,
            source_session,
            clock_namespace,
            executable_identity,
            binding: contract.binding,
            boot_id,
            contract_digest: stutter_core::hash::sha256_hex(&serde_json::to_vec(contract)?),
            from_ns: request.from_ns,
            to_ns: request.to_ns,
            samples,
        };
        block.recompute(contract)?;
        Ok(block)
    }

    /// Reconstruct every metric from the pinned raw samples. Supplied confidence intervals or
    /// serialized WindowAggregate values are never accepted as measurements.
    pub fn recompute(
        &self,
        contract: &KpiContract,
    ) -> anyhow::Result<BTreeMap<String, WindowAggregate>> {
        contract
            .validate()
            .map_err(|error| anyhow::anyhow!("invalid contract: {error:?}"))?;
        anyhow::ensure!(
            self.contract_digest == stutter_core::hash::sha256_hex(&serde_json::to_vec(contract)?)
                && self.binding == contract.binding,
            "block contract or process differs from the frozen contract"
        );
        let mut store = SampleStore::new(contract.clone());
        for sample in &self.samples {
            store
                .ingest_line(
                    crate::telemetry::registration::PeerCredentials {
                        pid: self.binding.pid,
                        uid: self.binding.uid,
                    },
                    self.binding.start_ticks,
                    &serde_json::to_string(sample)?,
                    sample.monotonic_ns,
                )
                .map_err(|error| anyhow::anyhow!("invalid raw sample: {error:?}"))?;
        }
        let mut values = BTreeMap::new();
        for spec in std::iter::once(&contract.primary).chain(&contract.hard_guardrails) {
            let aggregate = store
                .window(&spec.id, self.from_ns, self.to_ns, self.to_ns)
                .ok_or_else(|| anyhow::anyhow!("raw block has no required KPI {}", spec.id))?;
            anyhow::ensure!(
                aggregate.complete && aggregate.semantics == Semantics::Direct,
                "raw block lacks complete compatible KPI {}",
                spec.id
            );
            values.insert(spec.id.clone(), aggregate);
        }
        Ok(values)
    }

    pub fn artifact_bytes(&self) -> anyhow::Result<Vec<u8>> {
        Ok(serde_json::to_vec(self)?)
    }
    pub fn artifact_sha256(&self) -> anyhow::Result<String> {
        Ok(stutter_core::hash::sha256_hex(&self.artifact_bytes()?))
    }
    pub fn block_id(&self) -> &str {
        &self.block_id
    }
    pub fn run_id(&self) -> &str {
        &self.run_id
    }
    pub fn work_unit(&self) -> &str {
        &self.work_unit
    }
    pub fn context(&self) -> &str {
        &self.context_fingerprint
    }
    pub fn source_session(&self) -> &str {
        &self.source_session
    }
    pub fn interval(&self) -> (u64, u64) {
        (self.from_ns, self.to_ns)
    }
}
