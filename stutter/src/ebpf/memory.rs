//! Memory and page-size helpers for eBPF map sizing.

use std::{fs, path::Path};

pub(crate) fn available_memory_bytes() -> Option<u64> {
    mem_available_bytes_at(Path::new("/proc/meminfo")).or_else(available_memory_bytes_from_sysconf)
}

fn mem_available_bytes_at(path: &Path) -> Option<u64> {
    let meminfo = fs::read_to_string(path).ok()?;
    parse_mem_available_bytes(&meminfo)
}

pub(crate) fn parse_mem_available_bytes(meminfo: &str) -> Option<u64> {
    meminfo.lines().find_map(|line| {
        let value = line.strip_prefix("MemAvailable:")?;
        let kib = value.split_whitespace().next()?.parse::<u64>().ok()?;
        kib.checked_mul(1024)
    })
}

fn available_memory_bytes_from_sysconf() -> Option<u64> {
    let pages = crate::syscall::get_avphys_pages()?;
    let page_size = crate::syscall::get_pagesize();
    pages.checked_mul(page_size)
}

pub(crate) fn system_page_size() -> u64 {
    crate::syscall::get_pagesize()
}

pub(crate) fn format_optional_bytes(value: Option<u64>) -> String {
    value
        .map(|bytes| bytes.to_string())
        .unwrap_or_else(|| "unknown_or_unlimited".to_owned())
}
