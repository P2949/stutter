//! The prefix observation an initialization study records beside itself, read by the live judgement and
//! by every independent verification of it.

/// The file an initialization study records its prefix observation in.
pub(crate) const INIT_RECEIPT: &str = "init-observation.json";

/// Whether a recorded observation says every listed arm's prefix was warm. Any other shape, or another set
/// of arms, proves nothing.
pub(crate) fn read_init_receipt(bytes: &[u8], arms: &[&str]) -> Option<bool> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let listed: Vec<&str> = value["arms"]
        .as_array()?
        .iter()
        .filter_map(|a| a.as_str())
        .collect();
    (listed == arms)
        .then(|| value["all_warm"].as_bool())
        .flatten()
}

/// The file a feature study records, per attempt, which treatment it ran and whether that treatment was
/// seen in the sighted game process's own environment.
pub(crate) const FEATURE_RECEIPT: &str = "feature-observation.json";

type Observed = std::collections::BTreeMap<u32, (super::Treatment, bool)>;

pub(crate) fn feature_receipt_bytes(observed: &Observed) -> anyhow::Result<Vec<u8>> {
    let attempts: Vec<_> = observed
        .iter()
        .map(|(slot, (treatment, reached))| {
            serde_json::json!({ "slot": slot, "treatment": treatment, "reached": reached })
        })
        .collect();
    Ok(serde_json::to_vec(
        &serde_json::json!({ "attempts": attempts }),
    )?)
}

/// What a recorded feature observation says, per slot; any other shape proves nothing.
pub(crate) fn read_feature_receipt(bytes: &[u8]) -> Option<Observed> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let mut observed = Observed::new();
    for item in value["attempts"].as_array()? {
        let slot = u32::try_from(item["slot"].as_u64()?).ok()?;
        let treatment: super::Treatment = serde_json::from_value(item["treatment"].clone()).ok()?;
        let reached = item["reached"].as_bool()?;
        if observed.insert(slot, (treatment, reached)).is_some() {
            return None;
        }
    }
    Some(observed)
}
