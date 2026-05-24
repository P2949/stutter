//! Block I/O tracepoint implementation helpers.
//!
//! Keeps request correlation and event emission out of the eBPF crate entrypoint file.

use aya_ebpf::{
    helpers::{bpf_get_current_pid_tgid, bpf_ktime_get_ns},
    macros::map,
    maps::LruHashMap,
    programs::TracePointContext,
};
use stutter_common::{
    BlockIoEvent, DROP_BLOCK_FALLBACK_KEY_COLLISION, DROP_BLOCK_START_INSERT_FAILED, EVENT_BLOCK_IO,
};

use crate::{
    increment_drop_counter, is_target_pid_or_current_cgroup,
    trace_offsets::{
        BLOCK_RQ_COMPLETE_NR_SECTOR_OFFSET, BLOCK_RQ_COMPLETE_RWBS_OFFSET,
        BLOCK_RQ_ISSUE_NR_SECTOR_OFFSET, BLOCK_RQ_ISSUE_RWBS_OFFSET, BLOCK_RQ_KEY_OFFSET,
    },
    trace_read::{read_u32, read_u64},
};

#[repr(C)]
#[derive(Clone, Copy)]
struct IoStart {
    ts: u64,
    tid: u32,
}

#[map]
static BLOCK_START: LruHashMap<u64, IoStart> =
    LruHashMap::<u64, IoStart>::with_max_entries(16384, 0);

#[inline(always)]
fn block_rq_fallback_key(
    ctx: &TracePointContext,
    sector: u64,
    dev: u32,
    nr_sector_offset: u32,
    rwbs_offset: u32,
) -> u64 {
    // Use a 64-bit mixed key of (sector, dev, nr_sector, rwbs) to minimize
    // collisions during the fallback correlation mode.
    let mut h = sector.wrapping_mul(11400714819323198485u64)
        ^ ((dev as u64).wrapping_mul(14029467366897019727u64));

    if nr_sector_offset != 0 {
        let nr_sector: u32 = unsafe { ctx.read_at(nr_sector_offset as usize).unwrap_or(0) };
        h ^= (nr_sector as u64).wrapping_mul(11500714819323198485u64);
    }

    if rwbs_offset != 0 {
        let rwbs: u64 = unsafe { ctx.read_at(rwbs_offset as usize).unwrap_or(0) };
        h ^= rwbs.wrapping_mul(11600714819323198485u64);
    }

    h
}

#[inline(always)]
pub(crate) fn try_block_rq_issue(ctx: TracePointContext) -> u32 {
    let mut dev: u32 = 0;
    if !read_u32(&ctx, 8, &mut dev) {
        return 1;
    }
    let mut sector: u64 = 0;
    if !read_u64(&ctx, 16, &mut sector) {
        return 1;
    }
    let tid = (bpf_get_current_pid_tgid() & 0xffff_ffff) as u32;
    // Only track starts for target tasks so unrelated system I/O cannot
    // evict target entries from the start LRU map.
    if !is_target_pid_or_current_cgroup(tid) {
        return 0;
    }
    let ts = unsafe { bpf_ktime_get_ns() };
    // If userspace detected a unique request pointer field (like `rq`), use it
    // as the primary key. Otherwise fall back to a multi-field metadata hash.
    let request_key_offset = unsafe { core::ptr::read_volatile(&raw const BLOCK_RQ_KEY_OFFSET) };
    let using_fallback_key = request_key_offset == 0;
    let key = if !using_fallback_key {
        unsafe { ctx.read_at::<u64>(request_key_offset as usize).unwrap_or(0) }
    } else {
        let nr_sector_offset =
            unsafe { core::ptr::read_volatile(&raw const BLOCK_RQ_ISSUE_NR_SECTOR_OFFSET) };
        let rwbs_offset =
            unsafe { core::ptr::read_volatile(&raw const BLOCK_RQ_ISSUE_RWBS_OFFSET) };

        block_rq_fallback_key(&ctx, sector, dev, nr_sector_offset, rwbs_offset)
    };

    if key != 0 && using_fallback_key && unsafe { BLOCK_START.get(key).is_some() } {
        increment_drop_counter(DROP_BLOCK_FALLBACK_KEY_COLLISION);
        let _ = BLOCK_START.remove(key);
        return 0;
    }

    if key != 0 && BLOCK_START.insert(key, IoStart { ts, tid }, 0).is_err() {
        increment_drop_counter(DROP_BLOCK_START_INSERT_FAILED);
    }

    0
}

#[inline(always)]
pub(crate) fn try_block_rq_complete(ctx: TracePointContext) -> u32 {
    let mut dev: u32 = 0;
    if !read_u32(&ctx, 8, &mut dev) {
        return 1;
    }
    let mut sector: u64 = 0;
    if !read_u64(&ctx, 16, &mut sector) {
        return 1;
    }
    let mut nr_sector: u32 = 0;
    if !read_u32(&ctx, 24, &mut nr_sector) {
        return 1;
    }

    let key = if unsafe { core::ptr::read_volatile(&raw const BLOCK_RQ_KEY_OFFSET) } != 0 {
        unsafe {
            ctx.read_at::<u64>(core::ptr::read_volatile(&raw const BLOCK_RQ_KEY_OFFSET) as usize)
                .unwrap_or(0)
        }
    } else {
        let nr_sector_offset =
            unsafe { core::ptr::read_volatile(&raw const BLOCK_RQ_COMPLETE_NR_SECTOR_OFFSET) };
        let rwbs_offset =
            unsafe { core::ptr::read_volatile(&raw const BLOCK_RQ_COMPLETE_RWBS_OFFSET) };

        block_rq_fallback_key(&ctx, sector, dev, nr_sector_offset, rwbs_offset)
    };

    let start = match if key != 0 {
        unsafe { BLOCK_START.get(key) }
    } else {
        None
    } {
        Some(s) => *s,
        None => return 0,
    };
    let _ = BLOCK_START.remove(key);

    let now = unsafe { bpf_ktime_get_ns() };
    let duration_ns = now.saturating_sub(start.ts);

    let rwbs_offset = unsafe { core::ptr::read_volatile(&raw const BLOCK_RQ_COMPLETE_RWBS_OFFSET) };
    let rwbs = if rwbs_offset != 0 {
        unsafe { ctx.read_at(rwbs_offset as usize).unwrap_or([0u8; 8]) }
    } else {
        [0u8; 8]
    };

    emit_ringbuf_event!(
        BlockIoEvent,
        return 0,
        BlockIoEvent {
            kind: EVENT_BLOCK_IO,
            tid: start.tid,
            dev,
            nr_sector,
            sector,
            duration_ns,
            timestamp_ns: now,
            rwbs,
        }
    );

    0
}
