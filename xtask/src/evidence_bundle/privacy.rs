//! Fail-closed privacy handling for publishable evidence.
//! Order is: sanitize (explicit, documented placeholders), then scan; any finding
//! after sanitization is an error. Values that carry scientific provenance
//! (kernel, distribution, CPU, commit and digests) are never redacted.
use std::collections::BTreeSet;

use super::sha256::sha256_hex;

/// Placeholders and what they replace; recorded in every receipt.
pub(crate) const PLACEHOLDERS: [(&str, &str); 6] = [
    ("<EVIDENCE_DIR>", "absolute directory of the raw evidence"),
    ("<CHECKOUT>", "absolute repository checkout path"),
    ("<HOME>", "home directory"),
    ("<TMP>", "temporary directory prefix"),
    ("<USER>", "local account name"),
    ("<HOST>", "local hostname"),
];
const EMAIL_PLACEHOLDER: &str = "<EMAIL>";

#[derive(Clone, Debug, Default)]
pub(crate) struct Redactor {
    paths: Vec<(String, &'static str)>,
    user: Option<String>,
    host: Option<String>,
}

fn is_token_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')
}

/// Replaces whole-token occurrences of `needle` (so `p29` does not hit `p2949`).
fn replace_token(text: &str, needle: &str, replacement: &str) -> String {
    if needle.len() < 3 {
        return text.to_owned();
    }
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while let Some(found) = text[index..].find(needle) {
        let start = index + found;
        let end = start + needle.len();
        let boundary_before = start == 0 || !is_token_char(bytes[start - 1]);
        let boundary_after = end == bytes.len() || !is_token_char(bytes[end]);
        out.push_str(&text[index..start]);
        out.push_str(if boundary_before && boundary_after {
            replacement
        } else {
            needle
        });
        index = end;
    }
    out.push_str(&text[index..]);
    out
}

fn replace_emails(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('@') {
        let local_start = rest[..at]
            .rfind(|c: char| !(c.is_ascii_alphanumeric() || "._%+-".contains(c)))
            .map_or(0, |index| index + 1);
        let domain = &rest[at + 1..];
        let domain_len = domain
            .find(|c: char| !(c.is_ascii_alphanumeric() || ".-".contains(c)))
            .unwrap_or(domain.len());
        let domain_part = domain[..domain_len].trim_end_matches('.');
        if local_start < at && domain_part.contains('.') && !domain_part.starts_with('.') {
            out.push_str(&rest[..local_start]);
            out.push_str(EMAIL_PLACEHOLDER);
            rest = &rest[at + 1 + domain_part.len()..];
        } else {
            out.push_str(&rest[..=at]);
            rest = &rest[at + 1..];
        }
    }
    out.push_str(rest);
    out
}

impl Redactor {
    /// Captures the local identity of the machine that produced the evidence.
    pub(crate) fn for_machine(
        raw_dir: &str,
        checkout: &str,
        home: Option<&str>,
        user: Option<&str>,
        host: Option<&str>,
        protected: &BTreeSet<String>,
    ) -> Self {
        let mut paths = vec![
            (raw_dir.to_owned(), "<EVIDENCE_DIR>"),
            (checkout.to_owned(), "<CHECKOUT>"),
        ];
        if let Some(home) = home.filter(|home| home.len() > 1) {
            paths.push((home.to_owned(), "<HOME>"));
        }
        paths.sort_by_key(|(path, _)| std::cmp::Reverse(path.len()));
        // A hostname equal to provenance (e.g. a distribution id) cannot be told apart.
        let keep = |value: Option<&str>| {
            value
                .filter(|value| value.len() >= 3 && !protected.contains(*value))
                .map(str::to_owned)
        };
        Self {
            paths,
            user: keep(user),
            host: keep(host),
        }
    }

    pub(crate) fn sanitize(&self, text: &str) -> String {
        let mut text = text.to_owned();
        for (path, placeholder) in &self.paths {
            text = text.replace(path.trim_end_matches('/'), placeholder);
        }
        text = text.replace("/tmp/", "<TMP>/");
        if let Some(user) = &self.user {
            text = replace_token(&text, user, "<USER>");
        }
        if let Some(host) = &self.host {
            text = replace_token(&text, host, "<HOST>");
        }
        replace_emails(&text)
    }

    /// Salted-free digests of the local identity tokens, so a verifier on another
    /// machine can still prove their absence without learning them.
    pub(crate) fn literal_hashes(&self) -> Vec<String> {
        self.user
            .iter()
            .chain(self.host.iter())
            .map(|literal| sha256_hex(literal.as_bytes()))
            .collect()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Finding {
    pub kind: &'static str,
    pub line: usize,
}

const CASE_INSENSITIVE_MARKERS: [(&str, &str); 22] = [
    ("password", "credential"),
    ("passwd", "credential"),
    ("secret_key", "credential"),
    ("api_key", "credential"),
    ("apikey", "credential"),
    ("private_key", "credential"),
    ("authorization: bearer", "credential"),
    ("ghp_", "credential"),
    ("github_pat_", "credential"),
    ("xoxb-", "credential"),
    ("-----begin", "key-material"),
    ("ssh-rsa", "ssh"),
    ("ssh-ed25519", "ssh"),
    ("ssh_auth_sock", "ssh"),
    ("known_hosts", "ssh"),
    ("id_rsa", "ssh"),
    ("id_ed25519", "ssh"),
    (".ssh/", "ssh"),
    ("/home/", "home-path"),
    ("/root/", "home-path"),
    ("/users/", "home-path"),
    ("/tmp/", "tmp-path"),
];
const ENVIRONMENT_KEYS: [&str; 11] = [
    "HOME=",
    "USER=",
    "LOGNAME=",
    "PWD=",
    "OLDPWD=",
    "XDG_RUNTIME_DIR=",
    "XAUTHORITY=",
    "DBUS_SESSION_BUS_ADDRESS=",
    "SHELL=",
    "HOSTNAME=",
    "PATH=",
];

fn contains_aws_key(line: &str) -> bool {
    line.match_indices("AKIA").any(|(start, _)| {
        line.as_bytes()
            .get(start + 4..start + 20)
            .is_some_and(|rest| {
                rest.iter()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
            })
    })
}

fn contains_email(line: &str) -> bool {
    replace_emails(line) != line
}

/// Scans already-sanitized text. `literal_hashes` are digests of local identity
/// tokens; every identifier-like token is hashed and compared.
pub(crate) fn scan(text: &str, literal_hashes: &BTreeSet<String>) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        let lower = line.to_ascii_lowercase();
        let mut push = |kind| {
            findings.push(Finding {
                kind,
                line: line_number,
            })
        };
        for (marker, kind) in CASE_INSENSITIVE_MARKERS {
            if lower.contains(marker) {
                push(kind);
            }
        }
        if ENVIRONMENT_KEYS.iter().any(|key| {
            line.match_indices(key).any(|(start, _)| {
                start == 0
                    || !line.as_bytes()[start - 1].is_ascii_alphanumeric()
                        && line.as_bytes()[start - 1] != b'_'
            })
        }) {
            push("environment-dump");
        }
        if contains_aws_key(line) {
            push("credential");
        }
        if line.contains("eyJ") && line.matches('.').count() >= 2 && line.contains("eyJ") {
            let token = line
                .split(|c: char| !(c.is_ascii_alphanumeric() || "._-".contains(c)))
                .any(|part| part.starts_with("eyJ") && part.matches('.').count() == 2);
            if token {
                push("credential");
            }
        }
        if contains_email(line) {
            push("email");
        }
        if !literal_hashes.is_empty()
            && line
                .split(|c: char| !(c.is_ascii_alphanumeric() || "_-.".contains(c)))
                .filter(|token| token.len() >= 3)
                .any(|token| literal_hashes.contains(&sha256_hex(token.as_bytes())))
        {
            push("local-identity");
        }
    }
    findings.sort_by_key(|finding| (finding.line, finding.kind));
    findings.dedup();
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn redactor() -> Redactor {
        Redactor::for_machine(
            "/tmp/run/evidence",
            "/home/alice/src/stutter",
            Some("/home/alice"),
            Some("alice"),
            Some("workstation"),
            &BTreeSet::from(["nixos".to_owned()]),
        )
    }

    #[test]
    fn sanitization_uses_documented_placeholders_and_whole_tokens() {
        let redactor = redactor();
        let text = "/tmp/run/evidence/a.json /home/alice/src/stutter/x /home/alice/.local alice \
                    workstation alicedata bob@example.org /tmp/other";
        let clean = redactor.sanitize(text);
        assert_eq!(
            clean,
            "<EVIDENCE_DIR>/a.json <CHECKOUT>/x <HOME>/.local <USER> <HOST> alicedata <EMAIL> <TMP>/other"
        );
        assert!(scan(&clean, &BTreeSet::new()).is_empty());
    }

    #[test]
    fn provenance_tokens_are_never_treated_as_the_hostname() {
        let redactor = Redactor::for_machine(
            "/r",
            "/c",
            None,
            None,
            Some("nixos"),
            &BTreeSet::from(["nixos".to_owned()]),
        );
        assert_eq!(redactor.sanitize("ID=nixos"), "ID=nixos");
        assert!(redactor.literal_hashes().is_empty());
    }

    #[test]
    fn scanner_rejects_every_forbidden_class() {
        let hashes = BTreeSet::from([sha256_hex(b"alice")]);
        for (text, kind) in [
            ("path /home/bob/x", "home-path"),
            ("path /root/x", "home-path"),
            ("scratch /tmp/x", "tmp-path"),
            ("mail someone@example.org", "email"),
            ("password=hunter2", "credential"),
            ("ghp_abcdefghijklmnopqrstuvwxyz0123456789", "credential"),
            ("key AKIAABCDEFGHIJKLMNOP", "credential"),
            ("-----BEGIN OPENSSH PRIVATE KEY-----", "key-material"),
            ("ssh-ed25519 AAAA", "ssh"),
            ("SSH_AUTH_SOCK=/run/x", "ssh"),
            ("HOME=/x", "environment-dump"),
            ("owner alice here", "local-identity"),
            ("jwt eyJhbGciOi.eyJzdWIiOi.c2lnbmF0dXJl", "credential"),
        ] {
            let findings = scan(text, &hashes);
            assert!(
                findings.iter().any(|finding| finding.kind == kind),
                "{text}: {findings:?}"
            );
        }
    }

    #[test]
    fn scanner_accepts_ordinary_evidence_text() {
        let text = "rollback_token present\nPATH_MAX\n\"token\": {\"kind\":\"cpu-affinity-restore-file\"}\n\
                    sched_setaffinity(1, 128, [0]) = 0\n/proc/12/task/12/status\nkernel 6.18.55\nsha 4c72c9c8";
        assert!(scan(text, &BTreeSet::new()).is_empty());
    }
}
