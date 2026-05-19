#![allow(dead_code)] // Transitional remote compatibility namespace.

pub(crate) fn normalize_legacy_safety_class(value: &str) -> &str {
    match value {
        "low" | "low-risk" => "reversible_low_risk",
        "medium" | "medium-risk" => "reversible_medium_risk",
        "high" | "high-risk" => "high_risk",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_legacy_safety_class_names() {
        assert_eq!(
            normalize_legacy_safety_class("low-risk"),
            "reversible_low_risk"
        );
        assert_eq!(normalize_legacy_safety_class("high-risk"), "high_risk");
    }
}
