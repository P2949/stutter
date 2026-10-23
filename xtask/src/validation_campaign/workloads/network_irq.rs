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

/// Source port every campaign flow uses (`0` would pick a random one): receive-side scaling
/// hashes the flow onto one queue, so the probe and every capture must share the 4-tuple or the
/// frozen IRQ would only rise by chance.
pub const FLOW_SOURCE_PORT: u16 = 47_831;

/// Bounded UDP sender: `pps` packets per second of `payload` bytes to `target` for `seconds`.
/// Returns the packets actually sent. Datagrams go to the discard port; nothing is exfiltrated.
pub fn send_traffic(
    target: SocketAddr,
    source_port: u16,
    pps: u32,
    payload: usize,
    seconds: u64,
) -> std::io::Result<u64> {
    let socket = UdpSocket::bind(("0.0.0.0", source_port))?;
    socket.connect(target)?;
    let buffer = vec![0_u8; payload.clamp(1, 1_400)];
    let interval = Duration::from_secs_f64(1.0 / f64::from(pps.max(1)));
    let (start, deadline) = (
        Instant::now(),
        Instant::now() + Duration::from_secs(seconds),
    );
    let mut sent = 0_u64;
    while Instant::now() < deadline {
        // Sleep-free pacing keeps the rate steady without drifting behind schedule.
        let due = start + interval.saturating_mul(u32::try_from(sent).unwrap_or(u32::MAX));
        if Instant::now() < due {
            std::thread::sleep(due - Instant::now());
        }
        if socket.send(&buffer).is_ok() {
            sent += 1;
        }
    }
    Ok(sent)
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
