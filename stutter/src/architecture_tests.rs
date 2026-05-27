use std::path::{Path, PathBuf};

mod allow_attributes;
mod allowlists;
mod artifact_paths;
mod autotune_facades;
mod autotune_focus_policy;
mod autotune_raw_score;
mod cgroup_imports;
mod concurrency;
mod daemon_state;
mod decode_coverage;
mod dependencies;
mod direct_prints;
mod documentation;
mod ebpf_layout;
mod ebpf_switch_accounting;
mod ebpf_wakeup_accounting;
mod file_size;
mod module_layout;
mod mutation_paths;
mod objectives;
mod panic_paths;
mod privileged_errors;
mod public_api;
mod raw_ids;
mod rolling_window_privacy;
mod scanners;
mod scratch_dir;
mod test_layout;
mod transitional;
mod transitional_allowlist;
mod typed_ids;
mod unsafe_safety;
mod unwrap_expect;

const PRODUCTION_RUST_FILE_SIZE_LIMIT_LINES: usize = 700;
const TEST_RUST_FILE_SIZE_LIMIT_LINES: usize = 1_000;

const WORKSPACE_SOURCE_ROOTS: &[&str] = &[
    "stutter/src",
    "stutter-ebpf/src",
    "stutter-common/src",
    "stutter-config/src",
    "stutter-core/src",
    "stutter-report/src",
    "xtask/src",
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("stutter crate should live under workspace root")
        .to_path_buf()
}

fn workspace_src_roots() -> Vec<PathBuf> {
    let workspace_root = workspace_root();

    WORKSPACE_SOURCE_ROOTS
        .iter()
        .map(|path| workspace_root.join(path))
        .collect()
}

fn relative_to_workspace_root(path: &Path) -> String {
    path.strip_prefix(workspace_root())
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn crate_src_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn autotune_src_root() -> PathBuf {
    crate_src_root().join("autotune")
}

fn relative_to_crate_root(path: &Path) -> String {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap();
    path.strip_prefix(manifest_dir)
        .or_else(|_| path.strip_prefix(workspace_root))
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
