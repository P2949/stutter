#[test]
fn public_ebpf_events_must_be_decodable() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let common_lib_path = manifest_dir
        .parent()
        .unwrap()
        .join("stutter-common")
        .join("src")
        .join("lib.rs");
    let decode_rs_path = manifest_dir.join("src").join("events").join("decode.rs");

    let common_lib = std::fs::read_to_string(&common_lib_path).unwrap();
    let decode_rs = std::fs::read_to_string(&decode_rs_path).unwrap();

    let required_events = [
        "EVENT_RUNNABLE_LATENCY",
        "EVENT_IRQ_LATENCY",
        "EVENT_MIGRATION",
        "EVENT_CPU_FREQ",
        "EVENT_STAT_WAIT",
        "EVENT_BLOCK_IO",
        "EVENT_EXEC",
        "EVENT_KMS_FLIP",
        "EVENT_DRM_FENCE",
    ];

    for event in required_events {
        assert!(
            common_lib.contains(event),
            "{} must exist in stutter-common/src/lib.rs",
            event
        );
        assert!(
            decode_rs.contains(&format!("{} =>", event)),
            "decode_ebpf_event must have a match arm for {}",
            event
        );
    }

    assert!(decode_rs.contains("StatWait("));
    assert!(decode_rs.contains("KmsFlip("));
    assert!(decode_rs.contains("DrmFence("));
}
