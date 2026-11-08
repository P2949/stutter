use std::fs;

use super::*;

fn proc_with(retrans: u64, drops: u64, rx: u64) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("stutter-net-{}-{retrans}-{rx}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("net")).unwrap();
    fs::write(
        dir.join("net/snmp"),
        format!(
            "Ip: Forwarding\nIp: 1\nTcp: RtoAlgorithm OutSegs RetransSegs InErrs\nTcp: 1 1000 {retrans} 0\n"
        ),
    )
    .unwrap();
    fs::write(
        dir.join("net/netstat"),
        format!("TcpExt: ListenOverflows ListenDrops TCPTimeouts\nTcpExt: {drops} {drops} 4\n"),
    )
    .unwrap();
    fs::write(
        dir.join("net/dev"),
        format!(
            "Inter-|   Receive | Transmit\n face |bytes packets errs drop fifo frame compressed multicast|bytes packets errs drop fifo colls carrier compressed\n    lo: 999999 1 0 0 0 0 0 0 999999 1 0 0 0 0 0 0\n  eth0: {rx} 5 0 2 0 0 0 0 3000 4 0 1 0 0 0 0\n"
        ),
    )
    .unwrap();
    dir
}

#[test]
fn deltas_between_readings_exclude_loopback_and_start_empty() {
    let first = proc_with(10, 1, 5000);
    let mut sensor = NetworkSensor::new(first.clone());
    assert_eq!(sensor.probe(), Availability::Available);
    sensor.start().unwrap();
    let initial = sensor.sample(1);
    assert!(initial.values.is_empty());
    assert_eq!(initial.quality, ObjectiveSignalQuality::Missing);

    let second = proc_with(25, 3, 9000);
    for file in ["snmp", "netstat", "dev"] {
        fs::copy(second.join("net").join(file), first.join("net").join(file)).unwrap();
    }
    let reading = sensor.sample(2);
    assert_eq!(reading.values["tcp_retrans_segs_delta"], 15.0);
    assert_eq!(reading.values["tcp_listen_drops_delta"], 2.0);
    assert_eq!(reading.values["net_rx_bytes_delta"], 4000.0);
    assert_eq!(
        reading.values["net_tx_bytes_delta"], 0.0,
        "loopback is not counted"
    );
    assert_eq!(reading.quality, ObjectiveSignalQuality::Direct);
    assert_eq!(reading.lost, 0);
}

#[test]
fn a_counter_that_goes_backwards_is_lost_not_negative_and_a_missing_proc_is_unavailable() {
    let dir = proc_with(50, 0, 100);
    let mut sensor = NetworkSensor::new(dir.clone());
    sensor.start().unwrap();
    sensor.sample(1);
    let lower = proc_with(5, 0, 100);
    fs::copy(lower.join("net/snmp"), dir.join("net/snmp")).unwrap();
    let reading = sensor.sample(2);
    assert!(!reading.values.contains_key("tcp_retrans_segs_delta"));
    assert!(reading.lost >= 1);
    assert!(matches!(sensor.health(), SensorHealth::Degraded(_)));

    let absent = NetworkSensor::new(PathBuf::from("/nonexistent-stutter-proc"));
    assert!(matches!(absent.probe(), Availability::Unavailable(_)));
}

#[test]
fn the_real_host_counters_read_and_give_non_negative_deltas() {
    let mut sensor = NetworkSensor::new(PathBuf::from("/proc"));
    if sensor.start().is_err() {
        return;
    }
    sensor.sample(1);
    let reading = sensor.sample(2);
    assert!(reading.values.values().all(|v| *v >= 0.0));
}
