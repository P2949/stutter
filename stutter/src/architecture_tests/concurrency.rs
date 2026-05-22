//! Architecture checks for concurrency model documentation.

#[test]
fn concurrency_model_documentation_covers_core_boundaries() {
    let docs = include_str!("../../../docs/CONCURRENCY.md");

    for required in [
        "DaemonStateStore",
        "tokio::spawn",
        "mpsc",
        "Mutex",
        "kernel/host mutation",
        "spawn_blocking",
    ] {
        assert!(
            docs.contains(required),
            "docs/CONCURRENCY.md must mention {required}"
        );
    }
}
