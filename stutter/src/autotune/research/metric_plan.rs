//! Which native metric a comparison is judged on. The 99th-percentile frame time is primary when it
//! can be: every completed attempt carries tail statistics computed from its verified frame file, so
//! the population is known. Otherwise the comparison falls back, as a whole, to the average FPS the
//! summary reports; a campaign is never judged on a mix of metrics, and an attempt without a known
//! population is never given a tail estimate. The choice is a pure function of the sealed evidence,
//! so a verifier recomputes it and gets the same answer; it is made before any comparison is read.

use std::collections::BTreeMap;

use super::{analysis::NativeMetric, report::AttemptEvidenceV1};

/// The metric every completed attempt of the campaign is judged on.
pub(crate) fn choose<'a>(items: impl IntoIterator<Item = &'a AttemptEvidenceV1>) -> NativeMetric {
    let mut any = false;
    for item in items {
        any = true;
        if tail_ms(item).is_none() {
            return NativeMetric::AverageFps;
        }
    }
    if any {
        NativeMetric::P99FrameMs
    } else {
        NativeMetric::AverageFps
    }
}

fn tail_ms(item: &AttemptEvidenceV1) -> Option<f64> {
    item.native_completion
        .as_ref()?
        .context
        .as_ref()?
        .frame_stats
        .map(|stats| stats.p99_frame_ms)
        .filter(|value| value.is_finite() && *value > 0.0)
}

/// An attempt's value under `metric`; `None` only when `metric` was not available for it.
pub(crate) fn value(item: &AttemptEvidenceV1, metric: NativeMetric) -> Option<f64> {
    match metric {
        NativeMetric::AverageFps => Some(item.average_fps),
        NativeMetric::P99FrameMs => tail_ms(item),
    }
}

/// Every attempt's value under the metric chosen for the whole campaign.
pub(crate) fn values(
    evidence: &BTreeMap<u32, AttemptEvidenceV1>,
    completed: impl IntoIterator<Item = u32>,
    frozen: Option<bool>,
) -> anyhow::Result<(NativeMetric, BTreeMap<u32, f64>)> {
    let completed: Vec<u32> = completed.into_iter().collect();
    let items = completed.iter().filter_map(|index| evidence.get(index));
    // A study run under a frozen confirmation rule keeps the metric its pilot froze, whatever its own
    // evidence would have chosen; evidence that cannot supply it is an error, never a silent switch.
    let metric = match frozen {
        Some(true) => NativeMetric::P99FrameMs,
        Some(false) => NativeMetric::AverageFps,
        None => choose(items),
    };
    let mut out = BTreeMap::new();
    for index in completed {
        let item = evidence
            .get(&index)
            .ok_or_else(|| anyhow::anyhow!("missing evidence for a completed attempt"))?;
        out.insert(
            index,
            value(item, metric)
                .ok_or_else(|| anyhow::anyhow!("the primary metric is unavailable"))?,
        );
    }
    Ok((metric, out))
}

/// A native completion's tail statistic as an FPS-equivalent (1000 / p99 frame ms), so that higher is
/// better everywhere selection and rule freezing compare runs; `None` without a known population.
fn tail_fps(native: &super::cyberpunk::CyberpunkBenchmarkCompletion) -> Option<f64> {
    let p99 = native.context.as_ref()?.frame_stats?.p99_frame_ms;
    (p99.is_finite() && p99 > 0.0).then(|| 1000.0 / p99)
}

/// The per-run values a study's selection and pilot rule are computed from, and whether they are the
/// tail metric: the 99th-percentile frame time (as FPS) when every run has a known population, else the
/// average FPS for all of them. The same function serves the live executor and every verifier, so the
/// two cannot disagree about which metric a study stood on.
pub(crate) fn run_values<'a>(
    natives: impl IntoIterator<Item = &'a super::cyberpunk::CyberpunkBenchmarkCompletion>,
) -> (bool, Vec<(super::ProtonSource, f64)>) {
    let natives: Vec<_> = natives.into_iter().collect();
    let tails: Option<Vec<_>> = natives
        .iter()
        .map(|native| tail_fps(native).map(|value| (native.proton_source, value)))
        .collect();
    match tails {
        Some(tails) if !tails.is_empty() => (true, tails),
        _ => (
            false,
            natives
                .iter()
                .map(|native| (native.proton_source, native.metrics.average_fps))
                .collect(),
        ),
    }
}

#[cfg(test)]
#[path = "metric_plan_tests.rs"]
pub(crate) mod tests;
