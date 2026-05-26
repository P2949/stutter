pub fn log_drop_counters(drop_counters: &crate::ebpf_loader::DropCountersSnapshot) {
    if drop_counters.total() == 0 {
        log::debug!("ebpf_drop_counters total=0");
        return;
    }

    log::warn!(
        "ebpf_drop_counters cumulative_total={} wakeup_data_insert_failed={} ringbuf_reserve_failed={} irq_start_times_insert_failed={} block_start_insert_failed={} block_fallback_key_collisions={} block_zero_keys={} drm_fence_missing_start={}",
        drop_counters.total(),
        drop_counters.wakeup_data_insert_failed,
        drop_counters.ringbuf_reserve_failed,
        drop_counters.irq_start_times_insert_failed,
        drop_counters.block_start_insert_failed,
        drop_counters.block_fallback_key_collisions,
        drop_counters.block_zero_keys,
        drop_counters.drm_fence_missing_start,
    );
}
