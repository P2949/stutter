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

/// The file a study records the settings digests its applied profile left, beside itself.
pub(crate) const SETTINGS_RECEIPT: &str = "applied-settings.json";

pub(crate) fn settings_receipt_bytes(digests: &[String]) -> anyhow::Result<Vec<u8>> {
    Ok(serde_json::to_vec(
        &serde_json::json!({ "sha256": digests }),
    )?)
}

/// The recorded digests; any other shape proves nothing (`None`).
pub(crate) fn read_settings_receipt(bytes: &[u8]) -> Option<Vec<String>> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let digests: Vec<String> = value["sha256"]
        .as_array()?
        .iter()
        .map(|d| d.as_str().map(str::to_owned))
        .collect::<Option<_>>()?;
    digests
        .iter()
        .all(|d| d.len() == 64 && d.bytes().all(|b| b.is_ascii_hexdigit()))
        .then_some(digests)
}

/// Whether a result ran with the settings the profile left: its sealed settings hash is one of them. With
/// no recorded digests (no profile was applied, or an older study) there is nothing to hold it to.
pub(crate) fn settings_conform(
    applied: &[String],
    native: &super::cyberpunk::CyberpunkBenchmarkCompletion,
) -> bool {
    applied.is_empty() || applied.contains(&native.settings_sha256)
}

/// One completed attempt's conformance to its study's frozen profile: a long session specifies none, a
/// result that ran with settings the applied profile did not leave violates it, and otherwise the game's own
/// reported context is judged. The live outcome and every verifier use this one function.
pub(crate) fn conformance_of(
    item: &super::report::AttemptEvidenceV1,
    applied: &[String],
    study: &super::child_study::ChildStudy,
) -> super::native_context::ProfileConformance {
    use super::native_context::ProfileConformance;
    if item.long_session.is_some() {
        return ProfileConformance::NotSpecified;
    }
    let Some(native) = item.native_completion.as_deref() else {
        return ProfileConformance::Unproven;
    };
    if !settings_conform(applied, native) {
        return ProfileConformance::Violates;
    }
    native
        .context
        .as_ref()
        .map_or(ProfileConformance::Unproven, |context| {
            context.conformance_in(study.profile, study.stage)
        })
}
