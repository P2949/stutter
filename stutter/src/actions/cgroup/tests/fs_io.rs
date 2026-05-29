use super::*;

#[test]
fn cgroup_fs_path_rejects_parent_traversal_in_token_paths() {
    let cgroup_root = temp_dir("cgroup-token-traversal");

    let err = cgroup_fs_path(&cgroup_root, Path::new("/old.slice/../escape.slice"))
        .unwrap_err()
        .to_string();

    assert!(err.contains("parent traversal"));
    fs::remove_dir_all(cgroup_root).ok();
}
