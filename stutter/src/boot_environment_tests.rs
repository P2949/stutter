use std::{fs, path::Path};

use super::*;

fn put(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn host(
    name: &str,
    cmdline: &str,
    version: &str,
    generation: &str,
) -> (std::path::PathBuf, BootEnvironment) {
    let base = std::env::temp_dir().join(format!("stutter-bootenv-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let (proc_root, sys, run) = (base.join("proc"), base.join("sys"), base.join("run"));
    put(&proc_root.join("cmdline"), cmdline);
    put(&proc_root.join("sys/kernel/version"), version);
    put(&proc_root.join("sys/kernel/osrelease"), "6.18.57");
    put(&proc_root.join("sys/kernel/random/boot_id"), "boot-a");
    put(
        &proc_root.join("cpuinfo"),
        "processor\t: 0\nmicrocode\t: 0x100\n",
    );
    fs::create_dir_all(&run).unwrap();
    std::os::unix::fs::symlink(
        format!("/nix/store/{generation}"),
        run.join("current-system"),
    )
    .unwrap();
    let environment = BootEnvironment::read_at(&proc_root, &sys, &run);
    (base, environment)
}

#[test]
fn the_preemption_model_comes_from_the_kernel_version_string() {
    assert_eq!(
        preempt_mode("#1-NixOS SMP PREEMPT_DYNAMIC Thu"),
        PreemptMode::Dynamic
    );
    assert_eq!(preempt_mode("#1 SMP PREEMPT_RT Fri"), PreemptMode::Rt);
    assert_eq!(preempt_mode("#1 SMP PREEMPT Fri"), PreemptMode::Preempt);
    assert_eq!(preempt_mode("#1 SMP Fri"), PreemptMode::NotPreemptible);
}

#[test]
fn isolation_parameters_parse_with_flags_and_ranges_and_bad_values_are_empty() {
    let isolation = parse_isolation(
        "root=x nohz_full=2-3,6 rcu_nocbs=2-3 isolcpus=domain,managed_irq,4-5 irqaffinity=0-1 quiet",
    );
    assert_eq!(isolation.nohz_full, vec![2, 3, 6]);
    assert_eq!(isolation.rcu_nocbs, vec![2, 3]);
    assert_eq!(
        isolation.isolcpus,
        vec![4, 5],
        "flags are dropped, the list kept"
    );
    assert_eq!(isolation.irqaffinity, vec![0, 1]);
    assert_eq!(parse_isolation("nohz_full=junk"), Isolation::default());
    assert_eq!(parse_isolation("quiet splash"), Isolation::default());
}

#[test]
fn the_environment_hash_follows_every_part_but_not_the_boot_id() {
    let (a, first) = host("a", "root=x quiet", "#1 SMP PREEMPT_DYNAMIC", "gen-1");
    let (b, same) = host("b", "root=x quiet", "#1 SMP PREEMPT_DYNAMIC", "gen-1");
    assert!(first.same_environment(&same.environment_hash));
    assert_eq!(first.microcode.as_deref(), Some("0x100"));
    assert_eq!(first.generation.as_deref(), Some("gen-1"));
    // A different boot id alone changes nothing: a reboot into the same setup is the same setup.
    let mut rebooted = same.clone();
    rebooted.boot_id = "boot-b".into();
    assert!(first.same_environment(&rebooted.environment_hash));
    for (name, cmdline, version, generation) in [
        (
            "cmd",
            "root=x quiet nohz_full=2-3",
            "#1 SMP PREEMPT_DYNAMIC",
            "gen-1",
        ),
        ("rt", "root=x quiet", "#1 SMP PREEMPT_RT", "gen-1"),
        ("gen", "root=x quiet", "#1 SMP PREEMPT_DYNAMIC", "gen-2"),
    ] {
        let (dir, other) = host(name, cmdline, version, generation);
        assert!(!first.same_environment(&other.environment_hash), "{name}");
        fs::remove_dir_all(dir).unwrap();
    }
    fs::remove_dir_all(a).unwrap();
    fs::remove_dir_all(b).unwrap();
}

#[test]
fn the_raw_command_line_is_never_stored_and_housekeeping_excludes_every_isolated_set() {
    let (dir, environment) = host(
        "priv",
        "root=UUID=secret-id nohz_full=2-3 isolcpus=5",
        "#1 SMP PREEMPT_DYNAMIC",
        "g",
    );
    let serialized = serde_json::to_string(&environment).unwrap();
    assert!(!serialized.contains("secret-id"), "{serialized}");
    assert_eq!(
        environment.housekeeping_cpus(&[0, 1, 2, 3, 4, 5]),
        vec![0, 1, 4]
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_rebuild_that_changes_only_the_generation_keeps_the_context_hash() {
    let (a, first) = host("ctx-a", "root=x quiet", "#1 SMP PREEMPT_DYNAMIC", "gen-1");
    let (b, other_generation) = host("ctx-b", "root=x quiet", "#1 SMP PREEMPT_DYNAMIC", "gen-2");
    assert_ne!(first.environment_hash, other_generation.environment_hash);
    assert_eq!(first.context_hash, other_generation.context_hash);
    let (c, isolated) = host(
        "ctx-c",
        "root=x quiet nohz_full=2",
        "#1 SMP PREEMPT_DYNAMIC",
        "gen-1",
    );
    assert_ne!(first.context_hash, isolated.context_hash);
    for dir in [a, b, c] {
        fs::remove_dir_all(dir).unwrap();
    }
}
