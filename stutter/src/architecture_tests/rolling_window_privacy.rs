#[test]
fn rolling_window_does_not_expose_mutable_internal_state() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source_path = manifest_dir
        .join("src")
        .join("autotune")
        .join("rolling_window")
        .join("mod.rs");

    let source = std::fs::read_to_string(&source_path).unwrap();

    let forbidden_public_fields = [
        "pub duration: Duration",
        "pub intervals: VecDeque",
        "pub frames: VecDeque",
        "pub diagnoses: VecDeque",
        "pub irq_events: VecDeque",
        "pub block_io_events: VecDeque",
        "pub gpu_samples: VecDeque",
        "pub cpu_freq_events: VecDeque",
        "pub foreground_events: VecDeque",
    ];

    for forbidden in forbidden_public_fields {
        assert!(
            !source.contains(forbidden),
            "RollingWindow must not expose mutable internal state directly: {forbidden}"
        );
    }

    let required_read_accessors = [
        "pub fn duration(&self) -> Duration",
        "pub fn intervals(&self) -> &VecDeque<IntervalRecord>",
        "pub fn frames(&self) -> &VecDeque<FrameEvent>",
        "pub fn diagnoses(&self) -> &VecDeque<LiveDiagnosisEntry>",
        "pub fn irq_events(&self) -> &VecDeque<IrqEventRecord>",
        "pub fn block_io_events(&self) -> &VecDeque<BlockIoRecord>",
        "pub fn gpu_samples(&self) -> &VecDeque<GpuSample>",
        "pub fn cpu_freq_events(&self) -> &VecDeque<CpuFreqRecord>",
        "pub fn foreground_events(&self) -> &VecDeque<ForegroundEvent>",
    ];

    for required in required_read_accessors {
        assert!(
            source.contains(required),
            "RollingWindow should expose read-only accessor: {required}"
        );
    }
}
