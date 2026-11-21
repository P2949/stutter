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
