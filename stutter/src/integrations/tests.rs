use std::fs;

use super::*;
use crate::autotune::operation::MutationOperationKind as Op;

fn root(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("stutter-owners-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn process(proc_root: &std::path::Path, pid: u32, comm: &str) {
    let dir = proc_root.join(pid.to_string());
    fs::create_dir_all(&dir).unwrap();
    // The kernel truncates comm to 15 bytes.
    let comm: String = comm.chars().take(15).collect();
    fs::write(dir.join("comm"), format!("{comm}\n")).unwrap();
}

fn sched_ext_state(sys: &std::path::Path, state: &str, ops: Option<&str>) {
    fs::create_dir_all(sys.join("kernel/sched_ext/root")).unwrap();
    fs::write(sys.join("kernel/sched_ext/state"), format!("{state}\n")).unwrap();
    if let Some(ops) = ops {
        fs::write(sys.join("kernel/sched_ext/root/ops"), format!("{ops}\n")).unwrap();
    }
}

#[test]
fn a_quiet_host_has_no_external_owner() {
    let base = root("quiet");
    let (proc_root, sys) = (base.join("proc"), base.join("sys"));
    fs::create_dir_all(&proc_root).unwrap();
    process(&proc_root, 10, "bash");
    sched_ext_state(&sys, "disabled", None);
    let report = probe(&proc_root, &sys);
    for domain in ALL_DOMAINS {
        assert_eq!(report.external_owner(domain), None, "{domain:?}");
    }
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn each_known_owner_claims_exactly_its_own_domains() {
    let cases: [(&str, ResourceDomain, &[ResourceDomain]); 11] = [
        (
            "gamemoded",
            D::TaskPriority,
            &[D::TaskPriority, D::CpuPower],
        ),
        ("lactd", D::GpuPower, &[D::GpuPower]),
        ("ananicy-cpp", D::TaskPriority, &[D::TaskPriority]),
        ("system76-scheduler", D::TaskPriority, &[D::TaskPriority]),
        ("tuned", D::VmKnobs, &[D::CpuPower, D::VmKnobs]),
        ("auto-cpufreq", D::CpuPower, &[D::CpuPower]),
        ("power-profiles-daemon", D::CpuPower, &[D::CpuPower]),
        ("irqbalance", D::IrqAffinity, &[D::IrqAffinity]),
        ("bpftune", D::Network, &[D::Network, D::VmKnobs]),
        ("numad", D::MemoryPlacement, &[D::MemoryPlacement]),
        ("scx_loader", D::Scheduler, &[D::Scheduler]),
    ];
    for (comm, sample, claimed) in cases {
        let base = root(comm);
        let (proc_root, sys) = (base.join("proc"), base.join("sys"));
        fs::create_dir_all(&proc_root).unwrap();
        process(&proc_root, 11, comm);
        sched_ext_state(&sys, "disabled", None);
        let report = probe(&proc_root, &sys);
        assert!(report.external_owner(sample).is_some(), "{comm}");
        for domain in ALL_DOMAINS {
            assert_eq!(
                report.external_owner(domain).is_some(),
                claimed.contains(&domain),
                "{comm} / {domain:?}"
            );
        }
        fs::remove_dir_all(&base).unwrap();
    }
}

#[test]
fn a_loaded_scheduler_owns_the_scheduler_even_without_a_manager() {
    let base = root("scx");
    let (proc_root, sys) = (base.join("proc"), base.join("sys"));
    fs::create_dir_all(&proc_root).unwrap();
    sched_ext_state(&sys, "enabled", Some("scx_lavd"));
    let report = probe(&proc_root, &sys);
    assert_eq!(
        report.sched_ext,
        SchedExt::Enabled {
            ops: "scx_lavd".into()
        }
    );
    assert!(report.external_owner(D::Scheduler).is_some());
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn the_oom_guards_are_reported_but_own_no_tuning_domain() {
    let base = root("oomd");
    let (proc_root, sys) = (base.join("proc"), base.join("sys"));
    fs::create_dir_all(&proc_root).unwrap();
    process(&proc_root, 12, "systemd-oomd");
    process(&proc_root, 13, "uresourced");
    let report = probe(&proc_root, &sys);
    assert_eq!(report.daemon(KnownOwner::SystemdOomd), Daemon::Running);
    assert_eq!(report.daemon(KnownOwner::Uresourced), Daemon::Running);
    for domain in ALL_DOMAINS {
        assert_eq!(report.external_owner(domain), None, "{domain:?}");
    }
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn unreadable_state_fails_closed() {
    let base = root("unknown");
    let sys = base.join("sys");
    sched_ext_state(&sys, "garbage", None);
    let report = probe(&base.join("missing-proc"), &sys);
    assert_eq!(report.sched_ext, SchedExt::Unknown);
    assert_eq!(report.daemon(KnownOwner::GameMode), Daemon::Unknown);
    for domain in ALL_DOMAINS {
        // Domains with no possible owner stay free; every claimed one is owned.
        let claimed =
            domain == D::Scheduler || OWNERS.iter().any(|spec| spec.domains.contains(&domain));
        assert_eq!(
            report.external_owner(domain).is_some(),
            claimed,
            "{domain:?}"
        );
    }
    let none = probe(&base.join("missing-proc"), &base.join("no-sys"));
    assert_eq!(none.sched_ext, SchedExt::Unsupported);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn an_owned_domain_denies_only_the_operations_that_change_it() {
    let base = root("deny");
    let (proc_root, sys) = (base.join("proc"), base.join("sys"));
    fs::create_dir_all(&proc_root).unwrap();
    for (pid, comm) in [
        (1, "gamemoded"),
        (2, "lactd"),
        (3, "irqbalance"),
        (4, "tuned"),
    ] {
        process(&proc_root, pid, comm);
    }
    let owned = probe(&proc_root, &sys);
    for op in [
        Op::Nice,
        Op::IoPrio,
        Op::CpuPowerEpp,
        Op::CpuPowerGovernor,
        Op::GpuPowerProfile,
        Op::GpuForcePerformance,
        Op::VmSwappiness,
        Op::VmDirtyRatio,
        Op::IrqKnownDevice,
    ] {
        assert!(owner_denial(Some(&owned), op).is_some(), "{op:?}");
    }
    // The Supported tuples are never blocked by another owner.
    for op in [Op::CpuAffinityProfile, Op::CgroupPlacement] {
        assert_eq!(owner_denial(Some(&owned), op), None, "{op:?}");
    }
    assert_eq!(owner_denial(None, Op::Nice), None);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn nix_wrapped_daemon_names_are_recognised() {
    let base = root("wrapped");
    let (proc_root, sys) = (base.join("proc"), base.join("sys"));
    fs::create_dir_all(&proc_root).unwrap();
    process(&proc_root, 20, ".lact-wrapped");
    process(&proc_root, 21, ".gamemoded-wrapped");
    let report = probe(&proc_root, &sys);
    assert_eq!(report.daemon(KnownOwner::Lact), Daemon::Running);
    assert_eq!(report.daemon(KnownOwner::GameMode), Daemon::Running);
    assert!(report.external_owner(D::GpuPower).is_some());
    fs::remove_dir_all(&base).unwrap();
}

mod receipts {
    use super::super::receipt::*;
    use crate::autotune::operation::MutationOperationKind as Op;

    const T: u64 = 1_000;

    fn key() -> ResourceKey {
        ResourceKey::Task {
            pid: 42,
            start_ticks: 7,
            attribute: TaskAttribute::Nice,
        }
    }

    fn receipt(owner: OwnerClass) -> ResourceReceipt {
        ResourceReceipt {
            key: key(),
            owner,
            source: "test".into(),
            observed_at_ns: T,
            expires_at_ns: T + 500,
            observed_value: "0".into(),
        }
    }

    #[test]
    fn only_a_fresh_matching_receipt_of_a_free_resource_permits_a_mutation() {
        for owner in [OwnerClass::StutterOwned, OwnerClass::UnmanagedVerified] {
            assert_eq!(
                receipt(owner).permits_mutation(&key(), T + 10, "0"),
                Ok(()),
                "{owner:?}"
            );
        }
        for owner in [
            OwnerClass::ExternalConfirmed,
            OwnerClass::ExternalSuspected,
            OwnerClass::UserExplicit,
            OwnerClass::SharedReadOnly,
            OwnerClass::Unknown,
        ] {
            assert_eq!(
                receipt(owner).permits_mutation(&key(), T + 10, "0"),
                Err(ReceiptDenial::Owner(owner))
            );
        }
    }

    #[test]
    fn staleness_drift_and_the_wrong_resource_all_deny() {
        let free = receipt(OwnerClass::UnmanagedVerified);
        assert_eq!(
            free.permits_mutation(&key(), T + 500, "0"),
            Err(ReceiptDenial::Expired)
        );
        assert_eq!(
            free.permits_mutation(&key(), T - 1, "0"),
            Err(ReceiptDenial::Expired)
        );
        assert_eq!(
            free.permits_mutation(&key(), T + 10, "5"),
            Err(ReceiptDenial::ExternalChange)
        );
        let other = ResourceKey::Task {
            pid: 42,
            start_ticks: 8,
            attribute: TaskAttribute::Nice,
        };
        assert_eq!(
            free.permits_mutation(&other, T + 10, "0"),
            Err(ReceiptDenial::WrongResource)
        );
        assert_eq!(free.drift("0"), Drift::None);
        assert_eq!(
            free.drift("9"),
            Drift::ExternalChange {
                expected: "0".into(),
                current: "9".into()
            }
        );
    }

    #[test]
    fn a_hint_never_permits_anything_and_absence_proves_nothing() {
        let present = receipt_from_hint(key(), true, "0", T, 500);
        let absent = receipt_from_hint(key(), false, "0", T, 500);
        assert_eq!(present.owner, OwnerClass::ExternalSuspected);
        assert_eq!(absent.owner, OwnerClass::Unknown);
        for hint in [&present, &absent] {
            assert!(hint.permits_mutation(&key(), T + 1, "0").is_err());
        }
    }

    #[test]
    fn the_book_is_bounded_keeps_the_newest_and_a_missing_receipt_denies() {
        let mut book = ReceiptBook::default();
        assert_eq!(book.permit(&key(), T, "0"), Err(ReceiptDenial::NoReceipt));
        book.record(receipt(OwnerClass::UnmanagedVerified));
        assert_eq!(book.permit(&key(), T + 1, "0"), Ok(()));
        // The same resource re-observed as externally owned replaces the old receipt.
        book.record(receipt(OwnerClass::ExternalConfirmed));
        assert_eq!(book.len(), 1);
        assert!(book.permit(&key(), T + 1, "0").is_err());
        for pid in 0..5000_u32 {
            book.record(ResourceReceipt {
                key: ResourceKey::Task {
                    pid,
                    start_ticks: 1,
                    attribute: TaskAttribute::Nice,
                },
                observed_at_ns: u64::from(pid),
                ..receipt(OwnerClass::Unknown)
            });
        }
        assert!(book.len() <= 4096);
    }

    #[test]
    fn the_supported_operations_need_no_receipt_and_every_other_family_does() {
        for op in [Op::CpuAffinityProfile, Op::CgroupPlacement, Op::Uclamp] {
            assert!(!operation_needs_receipt(op), "{op:?}");
        }
        for op in [
            Op::Nice,
            Op::IoPrio,
            Op::CpuPowerEpp,
            Op::GpuPowerProfile,
            Op::VmSwappiness,
            Op::IrqKnownDevice,
        ] {
            assert!(operation_needs_receipt(op), "{op:?}");
        }
        assert!(key().render().starts_with("task:42:7:"));
    }
}

mod adapter_reports {
    use std::fs;

    use super::super::{adapters::*, *};

    fn host(name: &str, comms: &[&str], sched: Option<&str>) -> OwnerReport {
        let base =
            std::env::temp_dir().join(format!("stutter-adapters-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let proc_root = base.join("proc");
        fs::create_dir_all(&proc_root).unwrap();
        for (index, comm) in comms.iter().enumerate() {
            let dir = proc_root.join((100 + index).to_string());
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("comm"), format!("{comm}\n")).unwrap();
        }
        let sys = base.join("sys");
        if let Some(state) = sched {
            fs::create_dir_all(sys.join("kernel/sched_ext")).unwrap();
            fs::write(sys.join("kernel/sched_ext/state"), format!("{state}\n")).unwrap();
        }
        let report = probe(&proc_root, &sys);
        fs::remove_dir_all(&base).unwrap();
        report
    }

    fn status(reports: &[AdapterReport], owner: KnownOwner) -> AdapterStatus {
        reports.iter().find(|r| r.owner == owner).unwrap().status
    }

    #[test]
    fn a_running_tool_is_a_conflict_on_its_domain_and_delegation_is_always_refused() {
        let reports = adapter_reports(&host(
            "conflict",
            &["gamemoded", ".lact-wrapped", "systemd-oomd"],
            Some("disabled"),
        ));
        assert_eq!(reports.len(), 13);
        assert_eq!(
            status(&reports, KnownOwner::GameMode),
            AdapterStatus::Conflict
        );
        assert_eq!(status(&reports, KnownOwner::Lact), AdapterStatus::Conflict);
        assert_eq!(
            status(&reports, KnownOwner::SystemdOomd),
            AdapterStatus::ObserveOnly
        );
        assert_eq!(
            status(&reports, KnownOwner::Tuned),
            AdapterStatus::Unavailable
        );
        let lact = reports
            .iter()
            .find(|r| r.owner == KnownOwner::Lact)
            .unwrap();
        assert_eq!(lact.conflicting_domain, Some(ResourceDomain::GpuPower));
        for report in &reports {
            assert!(
                report.delegation.starts_with("refused"),
                "{:?}",
                report.owner
            );
        }
        // D-Bus tools are named, with the reason no delegation exists.
        let scx = reports
            .iter()
            .find(|r| r.owner == KnownOwner::ScxLoader)
            .unwrap();
        assert_eq!(scx.interface, Interface::Dbus("org.scx.Loader"));
        assert!(scx.delegation.contains("D-Bus"));
    }

    #[test]
    fn an_unmanaged_scheduler_is_delegable_by_an_administrator_and_unknown_state_fails_closed() {
        let free = adapter_reports(&host("scx-free", &[], Some("disabled")));
        assert_eq!(
            status(&free, KnownOwner::ScxLoader),
            AdapterStatus::DelegableManual
        );
        let unsupported = adapter_reports(&host("scx-none", &[], None));
        assert_eq!(
            status(&unsupported, KnownOwner::ScxLoader),
            AdapterStatus::Unavailable
        );
        let blind = OwnerReport {
            sched_ext: SchedExt::Unknown,
            daemons: owner_domains()
                .into_iter()
                .map(|(owner, _)| (owner, Daemon::Unknown))
                .collect(),
        };
        assert!(
            adapter_reports(&blind)
                .iter()
                .all(|r| r.status == AdapterStatus::Unknown)
        );
    }
}

#[test]
fn a_real_owner_probe_and_context_signature_cost_little_on_this_host() {
    let started = std::time::Instant::now();
    let rounds = 20;
    for _ in 0..rounds {
        let _ = probe(std::path::Path::new("/proc"), std::path::Path::new("/sys"));
    }
    let per_probe = started.elapsed() / rounds;
    log::info!("owner probe: {per_probe:?} per scan of the real /proc");
    // It runs at most once per 30 seconds; even 50 ms would be far below one percent of a core.
    assert!(
        per_probe < std::time::Duration::from_millis(50),
        "{per_probe:?}"
    );
}
