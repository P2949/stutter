use std::{
    cell::RefCell,
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use super::{Scratch, scenarios_set};
use crate::validation_campaign::workloads::{
    CommandSpec, RUNTIME_TOOLS, ToolHost, fio, fixed_compile, identify, network_irq,
    preflight_tools, vkmark,
};

#[derive(Default)]
struct FakeTools {
    present: Vec<&'static str>,
    outputs: BTreeMap<String, Result<String, String>>,
    env: BTreeMap<String, String>,
    files: BTreeMap<String, String>,
    calls: RefCell<Vec<String>>,
}

impl FakeTools {
    fn tool(mut self, name: &'static str, version: &str) -> Self {
        self.present.push(name);
        self.outputs.insert(name.to_owned(), Ok(version.to_owned()));
        self
    }
}

impl ToolHost for FakeTools {
    fn find(&self, tool: &str) -> Option<PathBuf> {
        self.present
            .contains(&tool)
            .then(|| PathBuf::from(format!("/nix/store/x/bin/{tool}")))
    }
    fn output(&self, spec: &CommandSpec) -> Result<String, String> {
        self.calls
            .borrow_mut()
            .push(format!("{} {}", spec.program, spec.args.join(" ")));
        let key = if spec.args == ["--list-scenes"] {
            format!("{} --list-scenes", spec.program)
        } else {
            spec.program.clone()
        };
        self.outputs
            .get(&key)
            .cloned()
            .unwrap_or_else(|| Ok(String::new()))
    }
    fn env_var(&self, name: &str) -> Option<String> {
        self.env.get(name).cloned()
    }
    fn read_file(&self, path: &str) -> Result<String, String> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| format!("no file {path}"))
    }
    fn executable_sha256(&self, path: &std::path::Path) -> Option<String> {
        Some(crate::evidence_bundle::sha256::sha256_hex(
            path.to_string_lossy().as_bytes(),
        ))
    }
}

#[test]
fn tools_are_identified_by_version_and_digest_and_missing_ones_are_all_reported() {
    let host = FakeTools::default()
        .tool("vkmark", "vkmark 2025.01\nextra")
        .tool("mangohud", "v0.7.2");
    let identity = identify(&host, "vkmark", &["--version"]).unwrap();
    assert_eq!(identity.version, "vkmark 2025.01");
    assert_eq!(
        identity.sha256.len(),
        64,
        "the executable digest must be recorded"
    );
    let problems = preflight_tools(&host, &["vkmark", "fio", "gh", "unknown-tool"]).unwrap_err();
    assert_eq!(problems.len(), 3, "{problems:?}");
    assert!(problems.iter().any(|p| p.contains("fio is not on PATH")));
    assert!(
        problems
            .iter()
            .any(|p| p.contains("not a known campaign tool"))
    );
    assert!(preflight_tools(&host, &["vkmark", "mangohud"]).is_ok());
    assert_eq!(RUNTIME_TOOLS.len(), 7);
}

#[test]
fn the_vkmark_command_is_deterministic_and_logs_every_frame() {
    let set = scenarios_set();
    let light = vkmark::VkmarkConfig::light(&set.vkmark);
    let command = vkmark::command(&light, 30, Path::new("/campaign/mango"));
    assert_eq!(command.program, "mangohud");
    assert_eq!(
        command.args,
        [
            "vkmark",
            "-b",
            "cube:duration=30.0",
            "-s",
            "640x480",
            "-p",
            "fifo"
        ]
    );
    let config = &command.env[0];
    assert_eq!(config.0, "MANGOHUD_CONFIG");
    for part in [
        "autostart_log=1",
        "log_duration=30",
        "log_interval=0",
        "output_folder=/campaign/mango",
    ] {
        assert!(config.1.contains(part), "{}", config.1);
    }
    assert_eq!(light.identity(), "cube@640x480/fifo");
    let heavy = vkmark::VkmarkConfig::heavy(&set.vkmark);
    assert_ne!(
        vkmark::command(&heavy, 30, Path::new("/x")).digest(),
        command.digest()
    );
    assert_eq!(
        command.digest(),
        vkmark::command(&light, 30, Path::new("/other")).digest(),
        "the log folder is not part of the identity"
    );
}

#[test]
fn vkmark_preflight_needs_the_tools_a_display_and_the_frozen_scene() {
    let mut host = FakeTools::default()
        .tool("vkmark", "x")
        .tool("mangohud", "y");
    host.outputs.insert(
        "vkmark --list-scenes".to_owned(),
        Ok("[Scene] cube\n  [Option] duration\n    Description  : seconds\n[Scene] shading\n  [Option] shading\n[Scene] clear\n".to_owned()),
    );
    assert!(
        vkmark::preflight(&host, &["cube"])
            .unwrap_err()
            .contains("no display")
    );
    host.env
        .insert("WAYLAND_DISPLAY".to_owned(), "wayland-1".to_owned());
    let facts = vkmark::preflight(&host, &["cube", "shading"]).unwrap();
    assert_eq!(
        (facts.window_system.as_str(), facts.scenes.len()),
        ("wayland", 3)
    );
    assert!(
        vkmark::preflight(&host, &["nonexistent"])
            .unwrap_err()
            .contains("no scene nonexistent")
    );
    let missing = FakeTools::default().tool("vkmark", "x");
    assert!(
        vkmark::preflight(&missing, &["cube"])
            .unwrap_err()
            .contains("mangohud")
    );
}

#[test]
fn the_newest_mangohud_log_is_the_one_in_use() {
    let scratch = Scratch::new("mango");
    assert!(vkmark::newest_log(scratch.path()).is_none());
    std::fs::write(scratch.path().join("old.csv"), "a").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(30));
    std::fs::write(scratch.path().join("new.csv"), "b").unwrap();
    std::fs::write(scratch.path().join("ignore.txt"), "c").unwrap();
    assert_eq!(
        vkmark::newest_log(scratch.path())
            .unwrap()
            .file_name()
            .unwrap(),
        "new.csv"
    );
}

#[test]
fn fio_commands_separate_the_modest_target_from_the_blocking_intervention() {
    let dir = Path::new("/scratch");
    let target = fio::target_command(&fio::FioProfile::target(), dir, 30);
    let blocker = fio::blocker_command(&fio::FioProfile::blocker(), dir, 30);
    assert!(
        target.args.contains(&"--rw=randread".to_owned())
            && target.args.contains(&"--rate_iops=200".to_owned())
    );
    assert!(
        blocker.args.contains(&"--direct=1".to_owned())
            && blocker.args.contains(&"--fsync=1".to_owned())
    );
    assert!(!target.args.contains(&"--direct=1".to_owned()));
    assert!(target.args.iter().all(|a| !a.contains("--fsync")));
}

#[test]
fn fio_summaries_decide_whether_a_blocking_intervention_was_established() {
    let json = r#"{"jobs":[{"error":0,"read":{"total_ios":0,"clat_ns":{"percentile":{}}},
        "write":{"total_ios":900,"clat_ns":{"percentile":{"99.000000":8000000}}}}]}"#;
    let summary = fio::summarize(json).unwrap();
    assert_eq!(
        (summary.total_ios, summary.errors, summary.p99_clat_ns),
        (900, 0, Some(8_000_000))
    );
    fio::blocking_established(&summary, 500, 4_000_000).unwrap();
    assert!(
        fio::blocking_established(&summary, 5_000, 4_000_000)
            .unwrap_err()
            .contains("fewer")
    );
    assert!(
        fio::blocking_established(&summary, 500, 50_000_000)
            .unwrap_err()
            .contains("below")
    );
    let failed =
        fio::summarize(r#"{"jobs":[{"error":5,"read":{},"write":{"total_ios":900}}]}"#).unwrap();
    assert!(
        fio::blocking_established(&failed, 1, 1)
            .unwrap_err()
            .contains("errors")
    );
    assert!(fio::summarize("not json").is_err() && fio::summarize(r#"{"jobs":[]}"#).is_err());
}

const ROUTE: &str = "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\tMTU\tWindow\tIRTT\n\
eth0\t00000000\t0100A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n\
eth0\t0000A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0\n";

#[test]
fn the_default_route_gives_the_interface_and_gateway() {
    let (interface, gateway) = network_irq::default_route(ROUTE).unwrap();
    assert_eq!(
        (interface.as_str(), gateway.to_string().as_str()),
        ("eth0", "192.168.0.1")
    );
    assert!(network_irq::default_route("Iface\tDestination\tGateway\tFlags\n").is_none());
    // A down route is not a default route.
    assert!(network_irq::default_route(&ROUTE.replace("0003", "0002")).is_none());
}

const INTERRUPTS: &str = "           CPU0       CPU1\n  16:        100        200   IO-APIC  eth0\n  17:          5          5   PCI-MSI  other\nNMI:          3          4   Non-maskable\n";

#[test]
fn interrupt_counters_sum_across_cpus_and_select_the_source_that_rose() {
    let before = network_irq::parse_interrupts(INTERRUPTS);
    assert_eq!(before.get(&16), Some(&300));
    assert!(!before.contains_key(&0), "non-numeric rows are not IRQs");
    let after =
        network_irq::parse_interrupts(&INTERRUPTS.replace("100        200", "1100       1200"));
    assert_eq!(
        network_irq::best_irq(&before, &after, &[16, 17], 1_000),
        Some((16, 2_000))
    );
    assert_eq!(network_irq::best_irq(&before, &after, &[17], 1), None);
    assert_eq!(
        network_irq::counter_rose(&before, &after, 16, 1_500),
        Ok(2_000)
    );
    assert!(
        network_irq::counter_rose(&before, &after, 17, 10)
            .unwrap_err()
            .contains("rose by 0")
    );
    assert!(
        network_irq::counter_rose(&before, &after, 99, 1)
            .unwrap_err()
            .contains("not present")
    );
}

#[test]
fn no_gateway_or_no_msi_interrupts_means_no_controllable_irq_source() {
    let mut host = FakeTools::default();
    host.files
        .insert("/proc/net/route".to_owned(), ROUTE.to_owned());
    let source = network_irq::discover(&host, &|_| {
        vec!["16".to_owned(), "x".to_owned(), "17".to_owned()]
    })
    .unwrap();
    assert_eq!(
        (source.interface.as_str(), source.irqs.clone()),
        ("eth0", vec![16, 17])
    );
    assert!(
        network_irq::discover(&host, &|_| vec![])
            .unwrap_err()
            .contains("no independently controllable IRQ source")
    );
    let no_route = FakeTools::default();
    assert!(network_irq::discover(&no_route, &|_| vec![]).is_err());
}

#[test]
fn bounded_udp_traffic_is_rate_limited_and_stops() {
    let receiver = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let target = receiver.local_addr().unwrap();
    let started = std::time::Instant::now();
    let sent = network_irq::send_traffic(target, 0, 100, 64, 1).unwrap();
    assert!(started.elapsed().as_secs_f64() < 3.0);
    assert!(
        (50..=130).contains(&sent),
        "sent {sent} packets at 100 pps for 1 s"
    );
}

#[test]
fn the_fixed_compile_cycle_count_outlives_the_window_and_the_identity_is_pathless() {
    assert_eq!(
        fixed_compile::choose_cycles(10.0, 20, 4.0),
        3,
        "(20+4)*1.25/10 rounded up"
    );
    assert_eq!(fixed_compile::choose_cycles(100.0, 20, 4.0), 1);
    assert_eq!(fixed_compile::choose_cycles(0.0, 20, 4.0), 1);
    assert_eq!(fixed_compile::choose_cycles(f64::NAN, 20, 4.0), 1);
    let [clean, build] =
        fixed_compile::cycle_commands(Path::new("/campaign/workloads/fixed-compile-target"));
    assert_eq!(clean.args[..3], ["clean", "-p", "stutter-core"]);
    assert!(
        build.args.contains(&"--offline".to_owned())
            && build.args.contains(&"--release".to_owned())
    );
    assert!(
        build
            .args
            .iter()
            .any(|a| a == "/campaign/workloads/fixed-compile-target")
    );
    let identity = |cycles| {
        fixed_compile::identity(&fixed_compile::Identity {
            source_commit: "abc",
            cargo_lock_sha256: "def",
            cargo_version: "cargo 1",
            rustc_version: "rustc 1",
            cycles,
        })
    };
    let (a, b) = (identity(3), identity(4));
    assert_ne!(
        fixed_compile::workload_sha256(&a),
        fixed_compile::workload_sha256(&b),
        "N is part of the identity"
    );
    assert!(
        !a.to_string().contains("/campaign"),
        "no local paths in the identity"
    );
    assert_eq!(a["cycles"], 3);
}

#[test]
fn running_cycles_executes_clean_then_build_n_times_in_the_given_directory() {
    let host = FakeTools::default();
    let elapsed = fixed_compile::run_cycles(
        &host,
        Path::new("/repo"),
        Path::new("/t"),
        2,
        &std::time::Instant::now,
    )
    .unwrap();
    assert!(elapsed >= 0.0);
    let calls = host.calls.borrow();
    assert_eq!(calls.len(), 4);
    assert!(calls[0].starts_with("cargo clean") && calls[1].starts_with("cargo build"));
    assert_eq!(calls[0], calls[2]);
}

fn quick_plan() -> network_irq::FlowPlan {
    network_irq::FlowPlan {
        pps: 2_000,
        payload: 64,
        trial: std::time::Duration::from_millis(20),
        min_delta: 100,
    }
}

/// Counters that advance a queue per `read` pair: the reads alternate "before" and "after" of a
/// trial, and `winner(trial)` names the queue that rose in that trial.
fn scripted_reads(
    winner: impl Fn(usize) -> u32,
) -> impl Fn() -> Result<std::collections::BTreeMap<u32, u64>, String> {
    let calls = std::cell::Cell::new(0_usize);
    let totals = std::cell::RefCell::new(std::collections::BTreeMap::from([
        (1_u32, 0_u64),
        (2, 0),
        (3, 0),
    ]));
    move || {
        let n = calls.get();
        calls.set(n + 1);
        // Odd calls follow a pump: the trial's winner gains 500 interrupts.
        if n % 2 == 1 {
            *totals.borrow_mut().get_mut(&winner(n / 2)).unwrap() += 500;
        }
        Ok(totals.borrow().clone())
    }
}

#[test]
fn steering_keeps_only_a_flow_that_drove_the_wanted_irq() {
    let target = std::net::SocketAddr::from(([127, 0, 0, 1], 9));
    // Trials 0 and 1 land on other queues; trial 2 finally drives IRQ 3.
    let read = scripted_reads(|trial| if trial < 2 { 1 } else { 3 });
    assert!(network_irq::steer_flow(target, 3, 10, quick_plan(), &read).is_ok());
    let never = scripted_reads(|_| 1);
    let error = network_irq::steer_flow(target, 3, 3, quick_plan(), &never).unwrap_err();
    assert!(error.contains("no flow out of 3 drove IRQ 3"), "{error}");
}

#[test]
fn the_probed_irq_is_the_queue_fresh_flows_most_often_land_on() {
    let target = std::net::SocketAddr::from(([127, 0, 0, 1], 9));
    let read = scripted_reads(|trial| [2, 3, 2, 2, 1][trial % 5]);
    assert_eq!(
        network_irq::dominant_irq(target, &[1, 2, 3], 5, quick_plan(), &read),
        Some(2)
    );
    // Nothing responds: no controllable source.
    let quiet = || Ok(std::collections::BTreeMap::from([(1_u32, 0_u64), (2, 0)]));
    assert_eq!(
        network_irq::dominant_irq(target, &[1, 2], 3, quick_plan(), &quiet),
        None
    );
}

#[test]
fn the_gpu_stressor_is_bare_vkmark_while_the_recorded_target_goes_through_mangohud() {
    let params = scenarios_set().vkmark;
    let heavy = vkmark::VkmarkConfig::heavy(&params);
    let bare = vkmark::bare_command(&heavy, 40);
    assert_eq!(bare.program, "vkmark");
    assert!(bare.env.is_empty());
    assert_eq!(bare.args[..2], ["-b", "texture:duration=40.0"]);
    assert_eq!(bare.args[2..], ["-s", "3840x2160", "-p", "immediate"]);
    let logged = vkmark::command(&heavy, 40, std::path::Path::new("/m"));
    assert_eq!(
        (logged.program.as_str(), logged.args[0].as_str()),
        ("mangohud", "vkmark")
    );
}

#[test]
fn per_cpu_interrupt_counts_keep_each_cpu_apart() {
    let text = "           CPU0       CPU1       CPU2\n  45:         10         20       3000  IR-PCI-MSIX  enp5s0-tx-0\n  46:          0          0          0  IR-PCI-MSIX  enp5s0-tx-1\nNMI:          1          2          3  Non-maskable\n";
    let counts = network_irq::parse_interrupts_per_cpu(text);
    assert_eq!(counts.get(&45), Some(&vec![10, 20, 3000]));
    assert_eq!(counts.get(&46), Some(&vec![0, 0, 0]));
    assert_eq!(
        counts.len(),
        2,
        "named lines such as NMI are not numbered interrupts"
    );
}

#[test]
fn the_fixed_work_target_never_times_out_and_reports_how_long_the_work_took() {
    let command = fio::fixed_work_command(
        &fio::FioProfile::target_direct(),
        std::path::Path::new("/scratch"),
        123_456,
    );
    assert!(command.args.contains(&"--number_ios=123456".to_owned()));
    assert!(command.args.contains(&"--direct=1".to_owned()));
    assert!(
        !command
            .args
            .iter()
            .any(|a| a.contains("runtime") || a.contains("time_based")),
        "{:?}",
        command.args
    );
    let json = r#"{"jobs":[{"error":0,"job_runtime":21345,"read":{"total_ios":123456,"clat_ns":{"percentile":{"99.000000":412000}}}}]}"#;
    let summary = fio::fixed_work_summary(json).unwrap();
    assert_eq!(summary.elapsed_ms, 21345.0);
    assert_eq!(summary.total_ios, 123_456);
    assert_eq!(summary.p99_clat_ns, Some(412_000));
    assert!(fio::fixed_work_summary(r#"{"jobs":[{"error":5,"job_runtime":1}]}"#).is_err());
    assert!(fio::fixed_work_summary("not json").is_err());
}
