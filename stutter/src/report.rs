use std::{fs, path::Path};

use crate::{
    metrics::format_latency,
    recorder::{SESSION_SCHEMA_VERSION, SessionFile},
};

pub fn print_report(path: &Path, json: bool, top: usize) -> anyhow::Result<()> {
    let session_path = if path.is_dir() {
        path.join("session.json")
    } else {
        path.to_path_buf()
    };

    let data = fs::read_to_string(&session_path)?;
    let session: SessionFile = serde_json::from_str(&data)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&session)?);
        return Ok(());
    }

    println!("stutter report");
    println!("==============");
    println!("file: {}", session_path.display());
    println!("schema: {}", session.schema_version);
    println!("expected_schema: {}", SESSION_SCHEMA_VERSION);
    println!("run: {}", session.run_name.as_deref().unwrap_or("-"));
    println!("duration_ms: {}", session.duration_ms);
    println!("stop_reason: {}", session.stop_reason);
    println!("manual_pids: {:?}", session.config.manual_pids);
    println!("tree_roots: {:?}", session.config.tree_roots);
    println!("active_tasks_at_end: {}", session.active_target_pids_count);
    println!();

    let truncated = session
        .tasks
        .iter()
        .filter(|task| task.latency.truncated_samples > 0)
        .collect::<Vec<_>>();

    if !truncated.is_empty() {
        println!("percentile warnings");
        println!("-------------------");
        for task in truncated.iter().take(top) {
            println!(
                "task={} comm={} truncated_samples={} note=p95/p99 are capped; prefer max and over_1ms/over_2ms/over_5ms",
                task.task, task.comm, task.latency.truncated_samples
            );
        }
        println!();
    }

    let mut tasks = session
        .tasks
        .iter()
        .filter(|task| task.latency.samples > 0)
        .collect::<Vec<_>>();

    tasks.sort_by_key(|task| std::cmp::Reverse(task.latency.max_ns));

    println!("top tasks by max latency");
    println!("------------------------");
    for task in tasks.iter().take(top) {
        println!(
            "task={} active={} class={:?} comm={} process_pid={:?} samples={} max={} over_1ms={} over_2ms={} over_5ms={} percentile_scope={}",
            task.task,
            task.active,
            task.class,
            task.comm,
            task.process_pid,
            task.latency.samples,
            format_latency(task.latency.max_ns),
            task.latency.over_1ms,
            task.latency.over_2ms,
            task.latency.over_5ms,
            task.latency.percentile_scope,
        );
    }
    println!();

    tasks.sort_by_key(|task| {
        (
            std::cmp::Reverse(task.latency.over_5ms),
            std::cmp::Reverse(task.latency.over_2ms),
            std::cmp::Reverse(task.latency.over_1ms),
            std::cmp::Reverse(task.latency.max_ns),
        )
    });

    println!("top tasks by threshold counters");
    println!("-------------------------------");
    for task in tasks.iter().take(top) {
        println!(
            "task={} active={} class={:?} comm={} over_5ms={} over_2ms={} over_1ms={} max={}",
            task.task,
            task.active,
            task.class,
            task.comm,
            task.latency.over_5ms,
            task.latency.over_2ms,
            task.latency.over_1ms,
            format_latency(task.latency.max_ns),
        );
    }
    println!();

    println!("top spikes");
    println!("----------");
    for spike in session.top_spikes.iter().take(top) {
        println!(
            "task={} active={} class={:?} comm={} cpu={} latency={} wakeup_ns={} switch_ns={}",
            spike.task,
            spike.active,
            spike.class,
            spike.comm,
            spike.cpu,
            format_latency(spike.latency_ns),
            spike.wakeup_ns,
            spike.switch_ns,
        );
    }

    Ok(())
}
