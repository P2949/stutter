//! A cross-check of the native topology against an hwloc XML export (package W8).
//!
//! hwloc is never linked or run: an operator may export `lstopo --of xml` and this module reads
//! the `<object>` elements as data. The comparison is advisory. A disagreement is reported with
//! the exact difference and never rewrites the native topology, never raises a candidate's
//! validity, and never authorizes an operation. A malformed or oversize file is an error.
use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};

use crate::topology_v2::TopologyV2;

/// Largest export read, in bytes.
pub const MAX_XML_BYTES: usize = 4 * 1024 * 1024;
const MAX_OBJECTS: usize = 100_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HwlocObject {
    pub kind: String,
    pub cpus: Vec<u32>,
}

/// The value of attribute `name` inside one tag's text.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!(" {name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let rest = &tag[start..];
    Some(&rest[..rest.find('"')?])
}

/// An hwloc cpuset: comma-separated 32-bit hexadecimal words, most significant first.
fn cpuset(text: &str) -> Option<Vec<u32>> {
    let mut cpus = Vec::new();
    let words: Vec<&str> = text.split(',').collect();
    let total = words.len();
    for (index, word) in words.iter().enumerate() {
        let digits = word.trim().strip_prefix("0x")?;
        let value = u32::from_str_radix(digits, 16).ok()?;
        let base = u32::try_from((total - 1 - index) * 32).ok()?;
        cpus.extend(
            (0..32)
                .filter(|bit| value >> bit & 1 == 1)
                .map(|bit| base + bit),
        );
    }
    cpus.sort_unstable();
    Some(cpus)
}

/// Every `<object type=... cpuset=...>` in the export.
pub fn parse_objects(xml: &str) -> anyhow::Result<Vec<HwlocObject>> {
    ensure!(
        xml.len() <= MAX_XML_BYTES,
        "the export is over the size limit"
    );
    ensure!(xml.contains("<topology"), "not an hwloc topology export");
    let mut objects = Vec::new();
    for part in xml.split("<object").skip(1) {
        let tag = part.split('>').next().unwrap_or_default();
        let (Some(kind), Some(set)) = (attribute(tag, "type"), attribute(tag, "cpuset")) else {
            continue;
        };
        let cpus = cpuset(set).with_context(|| format!("bad cpuset {set:?}"))?;
        ensure!(objects.len() < MAX_OBJECTS, "too many objects");
        objects.push(HwlocObject {
            kind: kind.to_owned(),
            cpus,
        });
    }
    ensure!(
        !objects.is_empty(),
        "the export has no object with a cpuset"
    );
    Ok(objects)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrossCheck {
    pub agrees: bool,
    /// One line per disagreement, naming both sides.
    pub differences: Vec<String>,
}

/// Compares the native cache domains and the usable CPU set with hwloc's.
pub fn cross_check(native: &TopologyV2, objects: &[HwlocObject]) -> CrossCheck {
    let mut differences = Vec::new();
    for domain in &native.llc_domains {
        let kind = format!("L{}Cache", domain.level);
        let found = objects
            .iter()
            .any(|object| object.kind == kind && object.cpus == domain.cpus);
        if !found {
            differences.push(format!(
                "native {kind} over CPUs {:?} has no identical hwloc object",
                domain.cpus
            ));
        }
    }
    let machine: Vec<u32> = objects
        .iter()
        .find(|object| object.kind == "Machine")
        .map(|object| object.cpus.clone())
        .unwrap_or_default();
    if !machine.is_empty()
        && let Some(missing) = native
            .effective_cpus
            .iter()
            .find(|cpu| !machine.contains(cpu))
    {
        differences.push(format!(
            "native usable CPU {missing} is not in hwloc's Machine cpuset"
        ));
    }
    CrossCheck {
        agrees: differences.is_empty(),
        differences,
    }
}

#[cfg(test)]
#[path = "topology_hwloc_tests.rs"]
mod tests;
