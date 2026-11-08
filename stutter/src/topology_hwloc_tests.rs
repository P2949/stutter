use super::*;
use crate::topology_v2::{LlcDomain, TopologyV2};

fn native(llc: &[(u32, &[u32])], effective: &[u32]) -> TopologyV2 {
    TopologyV2 {
        llc_domains: llc
            .iter()
            .map(|(level, cpus)| LlcDomain {
                level: *level,
                cpus: cpus.to_vec(),
                size_kib: None,
            })
            .collect(),
        capacity_classes: Vec::new(),
        larger_cache_cpus: Vec::new(),
        die_groups: Default::default(),
        frequency_domains: Vec::new(),
        numa_distance: Default::default(),
        gpu_locality: Vec::new(),
        effective_cpus: effective.to_vec(),
        fingerprint: String::new(),
    }
}

const XML: &str = r#"<?xml version="1.0"?><topology version="2.0">
<object type="Machine" os_index="0" cpuset="0x000000ff">
 <object type="Package" cpuset="0x000000ff">
  <object type="L3Cache" cpuset="0x0000000f" cache_size="8388608" depth="3"/>
  <object type="L3Cache" cpuset="0x000000f0" cache_size="8388608" depth="3"/>
 </object></object></topology>"#;

#[test]
fn matching_cache_domains_agree_and_a_difference_is_named_not_applied() {
    let objects = parse_objects(XML).unwrap();
    assert_eq!(objects.len(), 4);
    let good = native(
        &[(3, &[0, 1, 2, 3]), (3, &[4, 5, 6, 7])],
        &[0, 1, 2, 3, 4, 5, 6, 7],
    );
    assert_eq!(
        cross_check(&good, &objects),
        CrossCheck {
            agrees: true,
            differences: vec![]
        }
    );
    let bad = native(&[(3, &[0, 1, 2, 3, 4, 5, 6, 7])], &[0, 1, 2, 3, 8]);
    let report = cross_check(&bad, &objects);
    assert!(!report.agrees);
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
    assert!(report.differences[1].contains("CPU 8"));
}

#[test]
fn wide_cpusets_decode_and_malformed_input_is_an_error() {
    assert_eq!(cpuset("0x00000001,0x80000000"), Some(vec![31, 32]));
    assert_eq!(cpuset("ff"), None);
    assert_eq!(cpuset("0xzz"), None);
    assert!(parse_objects("nothing").is_err());
    assert!(parse_objects("<topology></topology>").is_err());
    assert!(
        parse_objects(r#"<topology><object type="Machine" cpuset="0xq"/></topology>"#).is_err()
    );
    assert!(parse_objects(&format!("<topology>{}", " ".repeat(MAX_XML_BYTES))).is_err());
}
