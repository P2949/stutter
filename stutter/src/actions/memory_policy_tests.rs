use std::fs;

use super::*;

const MAPS: &str = "\
00400000 default file=/bin/app mapped=3 N0=3 kernelpagesize_kB=4\n\
7f0000000000 bind:0 anon=10 dirty=10 N0=10\n\
7f0000100000 interleave:0-1 anon=4 N0=2 N1=2\n\
7f0000200000 prefer:1 anon=1 N1=1\n\
7f0000300000 local anon=2 N0=2\n\
7ffc00000000 default stack anon=1 N0=1\n\
not-an-address bind:9\n\
7f0000400000 weirdpolicy:3 anon=1\n";

#[test]
fn the_policy_column_is_summarized_by_kind_with_the_nodes_it_names() {
    let observed = parse_policy(MAPS);
    assert_eq!(observed.mappings, 6);
    assert_eq!(
        observed.default_mappings, 3,
        "default, local, default stack"
    );
    assert_eq!(observed.bind_mappings, 1);
    assert_eq!(observed.interleave_mappings, 1);
    assert_eq!(observed.prefer_mappings, 1);
    assert_eq!(observed.named_nodes, BTreeSet::from([0, 1]));
}

#[test]
fn an_unreadable_process_is_none_and_a_readable_one_is_summarized() {
    let root = std::env::temp_dir().join(format!("stutter-mempolicy-{}", std::process::id()));
    fs::create_dir_all(root.join("42")).unwrap();
    fs::write(root.join("42/numa_maps"), MAPS).unwrap();
    assert_eq!(observe_policy(&root, 42).unwrap().mappings, 6);
    assert!(observe_policy(&root, 43).is_none());
    assert_eq!(parse_policy(""), ObservedPolicy::default());
}

#[test]
fn every_foreign_memory_policy_call_is_refused_with_the_reason_and_the_alternative() {
    for call in [
        MemoryPolicyCall::SetMempolicy,
        MemoryPolicyCall::Mbind,
        MemoryPolicyCall::SetMempolicyHomeNode,
    ] {
        let refusal = refuse_foreign_policy(call);
        assert_eq!(
            refusal,
            MemoryPolicyRefusal::ForeignProcessCannotBeTargeted { call }
        );
        let reason = refusal.reason();
        assert!(
            reason.contains("calling") && reason.contains("NumaMigration"),
            "{reason}"
        );
    }
}
