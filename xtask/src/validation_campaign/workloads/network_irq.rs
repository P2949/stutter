//! IRQ intervention: bounded UDP traffic toward the local default gateway so the physical NIC's
//! interrupt counter rises. The IRQ and rate are chosen during uncounted calibration (a short
//! probe must show a stable counter delta) and then frozen; a host with no independently
//! controllable physical IRQ source is `BlockedResource`, never filled with synthetic evidence.
use std::{
    collections::BTreeMap,
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    time::{Duration, Instant},
};

use super::ToolHost;

/// Default route as `(interface, gateway)` from `/proc/net/route`.
pub fn default_route(route_table: &str) -> Option<(String, Ipv4Addr)> {
    route_table.lines().skip(1).find_map(|line| {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 4 || fields[1] != "00000000" {
            return None;
        }
        let flags = u32::from_str_radix(fields[3], 16).ok()?;
        // RTF_UP (0x1) and RTF_GATEWAY (0x2).
        if flags & 0x3 != 0x3 {
            return None;
        }
        // The gateway is little-endian hex.
        let raw = u32::from_str_radix(fields[2], 16).ok()?;
        Some((fields[0].to_owned(), Ipv4Addr::from(raw.swap_bytes())))
    })
}

/// IRQ numbers of an interface from the names under `/sys/class/net/<if>/device/msi_irqs`.
pub fn parse_irq_names(names: &[String]) -> Vec<u32> {
    let mut irqs = names
        .iter()
        .filter_map(|name| name.parse::<u32>().ok())
        .collect::<Vec<_>>();
    irqs.sort_unstable();
    irqs
}

/// Total interrupt count per IRQ from `/proc/interrupts` (sum over CPU columns; numeric IRQs only).
pub fn parse_interrupts(text: &str) -> BTreeMap<u32, u64> {
    let mut lines = text.lines();
    let columns = lines
        .next()
        .map_or(0, |header| header.split_whitespace().count());
    lines
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let irq = fields.next()?.strip_suffix(':')?.parse::<u32>().ok()?;
            let total = fields
                .take(columns)
                .map(|c| c.parse::<u64>().unwrap_or(0))
                .sum();
            Some((irq, total))
        })
        .collect()
}

/// The candidate NIC IRQ whose counter rose the most between two reads, if any rose by at least
/// `min_delta`. Anything below the minimum is not an independently controllable source.
pub fn best_irq(
    before: &BTreeMap<u32, u64>,
    after: &BTreeMap<u32, u64>,
    candidates: &[u32],
    min_delta: u64,
) -> Option<(u32, u64)> {
    candidates
        .iter()
        .filter_map(|irq| Some((*irq, after.get(irq)?.checked_sub(*before.get(irq)?)?)))
        .filter(|(_, delta)| *delta >= min_delta)
        .max_by_key(|(_, delta)| *delta)
}

/// A positive capture counts only if the frozen IRQ's counter rose by at least `min_delta`.
pub fn counter_rose(
    before: &BTreeMap<u32, u64>,
    after: &BTreeMap<u32, u64>,
    irq: u32,
    min_delta: u64,
) -> Result<u64, String> {
    let (b, a) = (before.get(&irq).copied(), after.get(&irq).copied());
    match (b, a) {
        (Some(b), Some(a)) if a.saturating_sub(b) >= min_delta => Ok(a - b),
        (Some(b), Some(a)) => Err(format!(
            "IRQ {irq} counter rose by {} (< {min_delta})",
            a.saturating_sub(b)
        )),
        _ => Err(format!("IRQ {irq} is not present in /proc/interrupts")),
    }
}

/// A connected UDP socket toward `target`. Locally generated traffic is spread over the NIC's
/// transmit queues by a per-socket hash, so which interrupt a flow drives is decided when the
/// socket is created: callers steer by opening sockets until the wanted IRQ responds.
pub fn open_flow(target: SocketAddr, source_port: u16) -> std::io::Result<UdpSocket> {
    let socket = UdpSocket::bind(("0.0.0.0", source_port))?;
    socket.connect(target)?;
    Ok(socket)
}

/// Sends `pps` packets per second of `payload` bytes on `socket` for `duration`; returns the
/// packets actually sent. Datagrams go to the discard port; nothing is exfiltrated.
pub fn pump(socket: &UdpSocket, pps: u32, payload: usize, duration: Duration) -> u64 {
    let buffer = vec![0_u8; payload.clamp(1, 1_400)];
    let interval = Duration::from_secs_f64(1.0 / f64::from(pps.max(1)));
    let (start, deadline) = (Instant::now(), Instant::now() + duration);
    let mut sent = 0_u64;
    while Instant::now() < deadline {
        // Pacing against the start time keeps the rate steady without drifting behind schedule.
        let due = start + interval.saturating_mul(u32::try_from(sent).unwrap_or(u32::MAX));
        if Instant::now() < due {
            std::thread::sleep(due - Instant::now());
        }
        if socket.send(&buffer).is_ok() {
            sent += 1;
        }
    }
    sent
}

/// Bounded UDP sender on a fresh flow (see [`open_flow`]).
pub fn send_traffic(
    target: SocketAddr,
    source_port: u16,
    pps: u32,
    payload: usize,
    seconds: u64,
) -> std::io::Result<u64> {
    let socket = open_flow(target, source_port)?;
    Ok(pump(&socket, pps, payload, Duration::from_secs(seconds)))
}

/// Snapshot of `/proc/interrupts` (injectable for tests).
pub type ReadInterrupts<'a> = dyn Fn() -> Result<BTreeMap<u32, u64>, String> + 'a;

/// Tuning of one steering or probing pass.
#[derive(Clone, Copy, Debug)]
pub struct FlowPlan {
    pub pps: u32,
    pub payload: usize,
    pub trial: Duration,
    pub min_delta: u64,
}

/// Opens flows until one drives `irq` by at least `min_delta` within a trial, and returns that
/// socket so the caller keeps using exactly the flow that was verified.
pub fn steer_flow(
    target: SocketAddr,
    irq: u32,
    attempts: u32,
    plan: FlowPlan,
    read: &ReadInterrupts<'_>,
) -> Result<UdpSocket, String> {
    for _ in 0..attempts {
        let socket = open_flow(target, 0).map_err(|e| e.to_string())?;
        let before = read()?;
        pump(&socket, plan.pps, plan.payload, plan.trial);
        // Interrupt counters lag the packets that caused them slightly.
        std::thread::sleep(Duration::from_millis(100));
        if counter_rose(&before, &read()?, irq, plan.min_delta).is_ok() {
            return Ok(socket);
        }
    }
    Err(format!(
        "no flow out of {attempts} drove IRQ {irq} by {} in {:?}",
        plan.min_delta, plan.trial
    ))
}

/// The candidate that most often responds across `trials` fresh flows (ties go to the higher
/// IRQ number so the answer is deterministic): the queue a campaign can steer to cheaply.
pub fn dominant_irq(
    target: SocketAddr,
    candidates: &[u32],
    trials: u32,
    plan: FlowPlan,
    read: &ReadInterrupts<'_>,
) -> Option<u32> {
    let mut wins: BTreeMap<u32, u32> = BTreeMap::new();
    for _ in 0..trials {
        let Ok(socket) = open_flow(target, 0) else {
            continue;
        };
        let Ok(before) = read() else { continue };
        pump(&socket, plan.pps, plan.payload, plan.trial);
        std::thread::sleep(Duration::from_millis(100));
        let Ok(after) = read() else { continue };
        if let Some((irq, _)) = best_irq(&before, &after, candidates, plan.min_delta) {
            *wins.entry(irq).or_default() += 1;
        }
    }
    wins.into_iter()
        .max_by_key(|(irq, count)| (*count, *irq))
        .map(|(irq, _)| irq)
}

/// Gateway and candidate IRQs of the default interface, or why none is controllable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NicSource {
    pub interface: String,
    pub gateway: Ipv4Addr,
    pub irqs: Vec<u32>,
}

pub fn discover(
    host: &dyn ToolHost,
    msi_irq_names: &dyn Fn(&str) -> Vec<String>,
) -> Result<NicSource, String> {
    let route = host.read_file("/proc/net/route")?;
    let (interface, gateway) =
        default_route(&route).ok_or("no default IPv4 route: no gateway to drive traffic toward")?;
    let irqs = parse_irq_names(&msi_irq_names(&interface));
    if irqs.is_empty() {
        return Err(format!(
            "interface {interface} exposes no MSI interrupts: no independently controllable IRQ source"
        ));
    }
    Ok(NicSource {
        interface,
        gateway,
        irqs,
    })
}
