pub fn comm_to_string(comm: &[u8; 16]) -> String {
    std::str::from_utf8(comm)
        .unwrap_or("?")
        .trim_end_matches('\0')
        .to_owned()
}

pub fn format_latency(ns: u64) -> String {
    if ns >= 1_000_000 {
        format!("{:.3}ms", ns as f64 / 1_000_000.0)
    } else {
        format!("{:.3}us", ns as f64 / 1_000.0)
    }
}
