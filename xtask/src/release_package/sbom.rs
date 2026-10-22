//! Minimal CycloneDX SBOM derived from `Cargo.lock` (no external tooling).
use anyhow::Context;
use serde_json::{Value, json};

pub(super) fn cyclonedx_from_lock(lock: &str, version: &str) -> anyhow::Result<String> {
    let mut components = Vec::new();
    for block in lock.split("[[package]]").skip(1) {
        let field = |name: &str| {
            block.lines().find_map(|line| {
                line.strip_prefix(&format!("{name} = \""))
                    .and_then(|rest| rest.strip_suffix('"'))
            })
        };
        let (Some(name), Some(package_version)) = (field("name"), field("version")) else {
            continue;
        };
        let mut component = json!({
            "type": "library",
            "name": name,
            "version": package_version,
            "purl": format!("pkg:cargo/{name}@{package_version}"),
        });
        if let Some(checksum) = field("checksum") {
            component["hashes"] = json!([{"alg":"SHA-256","content":checksum}]);
        }
        if let Some(source) = field("source") {
            component["externalReferences"] = json!([{"type":"distribution","url":source}]);
        }
        components.push(component);
    }
    anyhow::ensure!(!components.is_empty(), "Cargo.lock lists no packages");
    components.sort_by_key(|c| {
        (
            c["name"].as_str().map(str::to_owned),
            c["version"].as_str().map(str::to_owned),
        )
    });
    let sbom: Value = json!({
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "version": 1,
        "metadata": {"component": {"type":"application","name":"stutter","version":version}},
        "components": components,
    });
    serde_json::to_string_pretty(&sbom).context("serialize SBOM")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_come_from_the_lockfile_with_checksums_and_sources() {
        let lock = "[[package]]\nname = \"b\"\nversion = \"2.0.0\"\nsource = \"registry+https://x\"\nchecksum = \"abcd\"\n\n\
                    [[package]]\nname = \"a\"\nversion = \"1.0.0\"\n";
        let sbom: Value =
            serde_json::from_str(&cyclonedx_from_lock(lock, "0.1.0").unwrap()).unwrap();
        let components = sbom["components"].as_array().unwrap();
        assert_eq!(components[0]["name"], "a");
        assert!(components[0].get("hashes").is_none());
        assert_eq!(components[1]["purl"], "pkg:cargo/b@2.0.0");
        assert_eq!(components[1]["hashes"][0]["content"], "abcd");
        assert!(cyclonedx_from_lock("", "0.1.0").is_err());
    }
}
