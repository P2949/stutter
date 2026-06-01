//! Architecture checks for concurrency model documentation and source invariants.

fn function_block<'a>(source: &'a str, signature: &str) -> &'a str {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("missing function signature `{signature}`"));

    let after_signature = &source[start..];
    let open_brace = after_signature
        .find('{')
        .unwrap_or_else(|| panic!("missing function body for `{signature}`"));

    let body_start = start + open_brace;
    let mut depth = 0usize;

    for (offset, ch) in source[body_start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth = depth
                    .checked_sub(1)
                    .unwrap_or_else(|| panic!("unbalanced braces while scanning `{signature}`"));

                if depth == 0 {
                    let end = body_start + offset + ch.len_utf8();
                    return &source[start..end];
                }
            }
            _ => {}
        }
    }

    panic!("unterminated function body for `{signature}`");
}

fn count_occurrences(source: &str, needle: &str) -> usize {
    source.match_indices(needle).count()
}

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

#[test]
fn daemon_state_store_keeps_single_owner_mutation_boundary() {
    let source = include_str!("../daemon/store.rs");

    assert!(
        source.contains("pub struct DaemonStateStore"),
        "daemon store should keep an explicit DaemonStateStore owner"
    );
    assert!(
        source.contains("state: DaemonState"),
        "DaemonStateStore should own DaemonState directly"
    );
    assert!(
        source.contains("fn mutate_current(&mut self"),
        "DaemonStateStore should keep a single in-place mutation boundary"
    );

    for forbidden in [
        "Arc<Mutex<DaemonState",
        "Mutex<DaemonState",
        "RwLock<DaemonState",
        "self.state.clone()",
    ] {
        assert!(
            !source.contains(forbidden),
            "DaemonStateStore should not reintroduce broad shared/cloned daemon state via `{forbidden}`"
        );
    }

    for signature in [
        "pub fn transition",
        "pub fn mark_fault",
        "pub fn pause",
        "pub fn resume",
        "pub fn mark_restored",
    ] {
        let block = function_block(source, signature);
        assert!(
            block.contains("self.mutate_current("),
            "{signature} should mutate daemon state through mutate_current()"
        );
    }
}

#[test]
fn watch_apply_host_mutation_runs_in_blocking_workers() {
    let source = include_str!("../watch/apply.rs");

    let one_shot_apply = function_block(source, "pub async fn apply_profile_command");
    assert!(
        one_shot_apply.contains("tokio::task::spawn_blocking(move ||")
            && one_shot_apply.contains("run_audited_action"),
        "apply_profile_command should run audited host mutation inside spawn_blocking"
    );

    for signature in [
        "pub async fn apply_profile_to_tree_blocking",
        "pub async fn apply_profile_to_tree_cached_blocking",
    ] {
        let block = function_block(source, signature);

        assert!(
            block.contains("tokio::task::spawn_blocking(move ||"),
            "{signature} should isolate blocking profile application with spawn_blocking"
        );
        assert!(
            block.contains("apply_cached_with_policy"),
            "{signature} should keep policy-checked profile application inside the blocking worker"
        );
        assert!(
            block.contains(".await"),
            "{signature} should await the blocking worker result"
        );
        assert!(
            block.contains("profile apply worker failed"),
            "{signature} should surface blocking worker join failures clearly"
        );
    }
}

#[test]
fn monitor_session_sensor_reads_use_blocking_workers() {
    let source = include_str!("../session/monitor_session/probes.rs");
    let block = function_block(source, "pub async fn handle_hwmon_tick");

    assert!(
        count_occurrences(block, "task::spawn_blocking") >= 2,
        "hwmon and GPU-engine sensor reads should use spawn_blocking workers"
    );
    assert!(
        block.contains("reader.sample(elapsed)"),
        "sensor readers should sample inside the blocking worker"
    );
    assert!(
        block.contains("hwmon worker failed"),
        "hwmon blocking worker failures should be reported"
    );
    assert!(
        block.contains("gpu engine worker failed"),
        "GPU-engine blocking worker failures should be reported"
    );
}
