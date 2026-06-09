pub fn normalize_identity_label(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

pub fn scenario_identity_hash(
    scenario_name: Option<&str>,
    workload_label: Option<&str>,
    route_label: Option<&str>,
) -> Option<String> {
    let scenario_name = normalize_identity_label(scenario_name)?;
    let mut parts = vec![format!("scenario={scenario_name}")];
    if let Some(workload_label) = normalize_identity_label(workload_label) {
        parts.push(format!("workload={workload_label}"));
    }
    if let Some(route_label) = normalize_identity_label(route_label) {
        parts.push(format!("route={route_label}"));
    }
    Some(stable_hash_hex(&parts))
}

fn stable_hash_hex(parts: &[String]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;

    for part in parts {
        for byte in part.len().to_string().as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }

        hash ^= b':' as u64;
        hash = hash.wrapping_mul(0x100000001b3);

        for byte in part.as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }

        hash ^= 0xff;
        hash = hash.wrapping_mul(0x100000001b3);
    }

    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenario_hash_uses_normalized_labels() {
        assert_eq!(
            scenario_identity_hash(Some(" city-run "), Some(" game "), Some(" route-a ")),
            scenario_identity_hash(Some("city-run"), Some("game"), Some("route-a"))
        );
    }

    #[test]
    fn scenario_hash_requires_scenario_name() {
        assert_eq!(
            scenario_identity_hash(None, Some("game"), Some("route-a")),
            None
        );
        assert_eq!(scenario_identity_hash(Some(""), Some("game"), None), None);
    }
}
