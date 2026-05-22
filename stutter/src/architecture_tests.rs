use std::path::{Path, PathBuf};

mod allow_attributes;
mod allowlists;
mod autotune_facades;
mod concurrency;
mod dependencies;
mod direct_prints;
mod file_size;
mod objectives;
mod public_api;
mod scanners;
mod transitional;
mod transitional_allowlist;
mod unwrap_expect;

const RUST_FILE_SIZE_LIMIT_LINES: usize = 1_000;

fn crate_src_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn autotune_src_root() -> PathBuf {
    crate_src_root().join("autotune")
}

fn relative_to_crate_root(path: &Path) -> String {
    path.strip_prefix(env!("CARGO_MANIFEST_DIR"))
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
