use std::collections::BTreeMap;

use crate::{
    metrics::{TaskStats, format_latency},
    process_tree::{TaskClass, TaskInfo},
};

pub fn render_status(
    active_targets: &BTreeMap<u32, TaskInfo>,
    stats: &BTreeMap<u32, TaskStats>,
) -> String {
    let mut by_class = BTreeMap::<TaskClass, (u64, u64)>::new();
    for stat in stats.values() {
        let entry = by_class.entry(stat.class).or_default();
        entry.0 = entry.0.saturating_add(stat.session_latency.count);
        entry.1 = entry.1.max(stat.session_latency.max_ns);
    }

    let mut output = format!(
        "stutter live active_tasks={} tracked_stats={}\n",
        active_targets.len(),
        stats.len()
    );

    for (class, (samples, max_ns)) in by_class {
        let bar_width = ((max_ns / 1_000_000).min(40)) as usize;
        output.push_str(&format!(
            "class={class:<12} samples={samples:<8} max={:<10} {}\n",
            format_latency(max_ns),
            "#".repeat(bar_width.max(1))
        ));
    }

    output.trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_class_status_lines() {
        let mut stats = BTreeMap::new();
        let mut task = TaskStats::new(7, "RenderThread".to_owned(), 0);
        task.class = TaskClass::Game;
        task.session_latency.record(2_000_000);
        stats.insert(7, task);

        let rendered = render_status(&BTreeMap::new(), &stats);

        assert!(rendered.contains("stutter live"));
        assert!(rendered.contains("class=Game"));
        assert!(rendered.contains("max=2.000ms"));
    }
}
