#[test]
fn cgroup_helper_modules_do_not_glob_import_parent_module() {
    let root = crate::architecture_tests::crate_src_root();
    for rel_path in [
        "actions/cgroup/fs_io.rs",
        "actions/cgroup/procfs.rs",
        "actions/cgroup/rollback.rs",
        "actions/cgroup/validation.rs",
    ] {
        let path = root.join(rel_path);
        let source = std::fs::read_to_string(&path).unwrap();
        assert!(
            !source.contains("use super::*;"),
            "{} should use explicit imports instead of use super::*;",
            path.display()
        );
    }
}
