#[test]
fn advisor_irq_recommendations_use_recorded_target_affinity_overlap() {
    let root = crate::architecture_tests::workspace_root();

    let models = std::fs::read_to_string(root.join("stutter/src/advisor/models.rs"))
        .expect("read advisor models");
    assert!(
        models.contains("AdvisorIrqAffinityOverlap")
            && models.contains("AdvisorTargetAffinityOverlap")
            && models.contains("irq_affinity_overlaps"),
        "advisor model should carry IRQ CPU / target affinity overlap context"
    );

    let engine = std::fs::read_to_string(root.join("stutter/src/advisor/engine.rs"))
        .expect("read advisor engine");
    assert!(
        engine.contains("irq_affinity_overlaps_from_analysis"),
        "advisor should derive IRQ affinity overlap from report analysis"
    );
    assert!(
        engine.contains("recorded target affinity")
            && engine.contains("moving IRQ")
            && engine.contains("away from CPU"),
        "IRQ advisor rationale should mention concrete CPU overlap guidance"
    );

    let session_files = std::fs::read_to_string(root.join("stutter/src/recorder/session_files.rs"))
        .expect("read session file model");
    assert!(
        session_files.contains("pub allowed_cpus: Option<String>"),
        "session tasks should persist recorded target CPU affinity"
    );
}
