//! Allocation-free ASCII case-insensitive string matching helpers.
//!
//! These helpers intentionally mirror `to_ascii_lowercase()`-based matching without
//! allocating per classification. Non-ASCII characters are compared exactly, matching
//! Rust's ASCII-only case folding behavior.

pub(crate) fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }

    let haystack = haystack.as_bytes();
    let needle = needle.as_bytes();
    if needle.len() > haystack.len() {
        return false;
    }

    haystack
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

pub(crate) fn starts_with_ignore_ascii_case(value: &str, prefix: &str) -> bool {
    let value = value.as_bytes();
    let prefix = prefix.as_bytes();
    value.len() >= prefix.len() && value[..prefix.len()].eq_ignore_ascii_case(prefix)
}

pub(crate) fn any_contains_ignore_ascii_case(fields: &[&str], needle: &str) -> bool {
    fields
        .iter()
        .any(|field| contains_ignore_ascii_case(field, needle))
}

pub(crate) fn normalized_eq_ignore_ascii_case(value: &str, expected: &str) -> bool {
    let mut value_chars = value
        .chars()
        .filter(|ch| *ch != '_' && *ch != '-' && !ch.is_whitespace());
    let mut expected_chars = expected.chars();

    loop {
        match (value_chars.next(), expected_chars.next()) {
            (Some(left), Some(right)) if left.eq_ignore_ascii_case(&right) => {}
            (None, None) => return true,
            _ => return false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AsciiCase<'a> {
    value: &'a str,
}

impl<'a> AsciiCase<'a> {
    pub(crate) fn new(value: &'a str) -> Self {
        Self { value }
    }

    pub(crate) fn contains(self, needle: &str) -> bool {
        contains_ignore_ascii_case(self.value, needle)
    }

    pub(crate) fn starts_with(self, prefix: &str) -> bool {
        starts_with_ignore_ascii_case(self.value, prefix)
    }
}

impl PartialEq<&str> for AsciiCase<'_> {
    fn eq(&self, other: &&str) -> bool {
        self.value.eq_ignore_ascii_case(other)
    }
}

#[cfg(test)]
mod tests {
    use super::{contains_ignore_ascii_case, normalized_eq_ignore_ascii_case};

    #[test]
    fn contains_matches_ascii_without_allocating_lowercase_copies() {
        assert!(contains_ignore_ascii_case("SteamApps/common", "steamapps"));
        assert!(contains_ignore_ascii_case("GPU-Process", "gpu-process"));
        assert!(!contains_ignore_ascii_case("renderer", "network"));
    }

    #[test]
    fn normalized_equality_ignores_common_class_name_separators() {
        assert!(normalized_eq_ignore_ascii_case(
            "browser-renderer",
            "browserrenderer"
        ));
        assert!(normalized_eq_ignore_ascii_case(
            "Audio_Realtime",
            "audiorealtime"
        ));
        assert!(!normalized_eq_ignore_ascii_case(
            "browser gpu",
            "browsernetwork"
        ));
    }
}
