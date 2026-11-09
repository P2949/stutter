use std::fs;

use super::*;

fn proc_root(
    name: &str,
    current: &str,
    available: &str,
    allowed: Option<&str>,
) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("stutter-netcap-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let ipv4 = root.join("sys/net/ipv4");
    fs::create_dir_all(&ipv4).unwrap();
    fs::write(ipv4.join("tcp_congestion_control"), format!("{current}\n")).unwrap();
    fs::write(
        ipv4.join("tcp_available_congestion_control"),
        format!("{available}\n"),
    )
    .unwrap();
    if let Some(allowed) = allowed {
        fs::write(
            ipv4.join("tcp_allowed_congestion_control"),
            format!("{allowed}\n"),
        )
        .unwrap();
    }
    root
}

#[test]
fn the_congestion_inventory_reports_current_available_and_allowed() {
    let root = proc_root("ok", "cubic", "reno cubic bbr", Some("reno cubic"));
    let inventory = read_congestion(&root).unwrap();
    assert_eq!(inventory.current, "cubic");
    assert_eq!(inventory.available, ["reno", "cubic", "bbr"]);
    assert_eq!(inventory.allowed, ["reno", "cubic"]);
}

#[test]
fn an_inconsistent_or_absent_inventory_is_none_not_a_guess() {
    // The running algorithm must be one the kernel lists as available.
    let root = proc_root("bad", "bbr", "reno cubic", None);
    assert!(read_congestion(&root).is_none());
    assert!(read_congestion(&root.join("nowhere")).is_none());
    let empty = proc_root("empty", "", "reno", None);
    assert!(read_congestion(&empty).is_none());
}

#[test]
fn tc_qdisc_output_parses_roots_and_ignores_everything_else() {
    let text = "qdisc noqueue 0: dev lo root refcnt 2\n\
qdisc fq_codel 0: dev eth0 root refcnt 2 limit 10240p flows 1024\n\
qdisc ingress ffff: dev eth0 parent ffff:fff1 ----------------\n\
this line is not a qdisc\n\
qdisc mq broken dev eth1 root\n";
    let entries = parse_tc_qdiscs(text);
    assert_eq!(entries.len(), 3, "{entries:?}");
    assert_eq!(entries[1].kind, "fq_codel");
    assert!(entries[1].root && entries[0].root);
    assert!(!entries[2].root, "ingress is not a root discipline");
    assert!(parse_tc_qdiscs("").is_empty());
}

#[test]
fn every_socket_option_and_every_qdisc_replacement_is_refused_with_a_reason() {
    for option in [
        SocketOption::BusyPoll,
        SocketOption::SendBuffer,
        SocketOption::ReceiveBuffer,
        SocketOption::Priority,
        SocketOption::TcpNoDelay,
        SocketOption::TcpCongestion,
    ] {
        let refusal = refuse_socket_option(option);
        assert_eq!(
            refusal,
            NetRefusal::SocketOptionNeedsOwningDescriptor { option }
        );
        assert!(refusal.reason().contains("setsockopt"));
    }
    let refusal = refuse_qdisc_replacement("eth0");
    assert!(refusal.reason().contains("eth0") && refusal.reason().contains("exactly"));
}
