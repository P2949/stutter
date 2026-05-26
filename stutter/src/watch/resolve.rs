use std::{path::Path, time::Duration};

use futures_util::future;
use log::info;
use tokio::{
    signal,
    time::{MissedTickBehavior, interval, sleep},
};

use super::{
    WatchProcessConfig, process_match::find_process_match_by_pattern_at_with_cache,
    tree_roots::add_watch_tree_pid,
};

pub async fn resolve_watch_process(
    watch: &WatchProcessConfig,
    tree_pids: &mut Vec<u32>,
) -> anyhow::Result<Option<u32>> {
    let Some(pattern) = watch.pattern.clone() else {
        return Ok(None);
    };

    let mut cache = crate::process_tree::ProcessCache::default();
    if let Some(decision) =
        find_process_match_by_pattern_at_with_cache(Path::new("/proc"), &pattern, &mut cache)
    {
        let pid = decision.pid.as_u32();
        add_watch_tree_pid(tree_pids, pid);
        info!(
            "watch_process_found pattern={} pid={} score={} reasons={:?}",
            pattern,
            pid,
            decision.score,
            decision.reason_labels()
        );
        return Ok(Some(pid));
    }

    wait_for_watch_process(watch, tree_pids)
        .await?
        .ok_or_else(|| anyhow::anyhow!("stopped while waiting for --watch-process {pattern}"))
        .map(Some)
}

pub(super) async fn wait_for_watch_process(
    watch: &WatchProcessConfig,
    tree_pids: &mut Vec<u32>,
) -> anyhow::Result<Option<u32>> {
    let pattern = watch
        .pattern
        .clone()
        .ok_or_else(|| anyhow::anyhow!("internal error: watch_process missing"))?;

    info!(
        "watch_process_waiting pattern={} persistent={}",
        pattern, watch.persistent
    );

    let mut tick = interval(Duration::from_millis(watch.poll_ms));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut cache = crate::process_tree::ProcessCache::default();

    let watch_timeout = watch.timeout;
    let timeout_future = async move {
        if let Some(timeout) = watch_timeout {
            sleep(timeout).await;
        } else {
            future::pending::<()>().await;
        }
    };
    tokio::pin!(timeout_future);

    loop {
        tokio::select! {
            _ = signal::ctrl_c() => {
                return Ok(None);
            }
            _ = &mut timeout_future => {
                anyhow::bail!(
                    "timed out waiting for --watch-process {pattern} after {}ms",
                    watch_timeout.map(|timeout| timeout.as_millis()).unwrap_or(0)
                );
            }
            _ = tick.tick() => {
                if let Some(decision) = find_process_match_by_pattern_at_with_cache(
                    Path::new("/proc"),
                    &pattern,
                    &mut cache,
                ) {
                    let pid = decision.pid.as_u32();
                    add_watch_tree_pid(tree_pids, pid);
                    info!(
                        "watch_process_found pattern={} pid={} score={} reasons={:?}",
                        pattern,
                        pid,
                        decision.score,
                        decision.reason_labels()
                    );
                    return Ok(Some(pid));
                }
            }
        }
    }
}
