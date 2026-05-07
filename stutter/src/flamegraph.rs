use std::{collections::BTreeMap, path::Path};

use anyhow::{Context, Result};

use crate::recorder::SpikeEvent;

pub fn write_latency_flamegraph_svg(spikes: &[SpikeEvent], output_path: &Path) -> Result<()> {
    if spikes.is_empty() {
        let svg = empty_latency_flamegraph_svg();
        std::fs::write(output_path, svg).with_context(|| {
            format!(
                "failed to write empty latency flamegraph SVG {}",
                output_path.display()
            )
        })?;
        return Ok(());
    }

    let folded = build_folded_latency_stacks(spikes);
    let svg = render_flamegraph_svg_from_folded_lines(&folded)?;
    std::fs::write(output_path, svg).with_context(|| {
        format!(
            "failed to write latency flamegraph SVG {}",
            output_path.display()
        )
    })?;
    Ok(())
}

fn empty_latency_flamegraph_svg() -> &'static str {
    r#"<svg xmlns="http://www.w3.org/2000/svg" width="800" height="120"><text x="20" y="40">No spike events available for latency flamegraph</text></svg>"#
}

pub fn sanitize_folded_label(value: &str) -> String {
    let mut out = String::with_capacity(value.len());

    for ch in value.chars() {
        match ch {
            ';' | '\n' | '\r' | '\t' => out.push('_'),
            c if c.is_whitespace() => out.push('_'),
            c => out.push(c),
        }
    }

    let out = out.trim_matches('_').to_string();

    if out.is_empty() {
        "unknown".to_string()
    } else {
        out
    }
}

pub fn build_folded_latency_stacks(spikes: &[SpikeEvent]) -> Vec<String> {
    let mut weights: BTreeMap<String, u64> = BTreeMap::new();

    for spike in spikes {
        let process = sanitize_folded_label(&spike.process_comm);
        let thread = sanitize_folded_label(&spike.comm);
        let cpu = spike.cpu;

        let latency_ns = spike.latency_ns;
        if latency_ns == 0 {
            continue;
        }

        let stack = format!("{process};{thread};cpu{cpu}");
        *weights.entry(stack).or_insert(0) += latency_ns;
    }

    weights
        .into_iter()
        .map(|(stack, weight)| format!("{stack} {weight}"))
        .collect()
}

pub fn render_flamegraph_svg_from_folded_lines(lines: &[String]) -> Result<String> {
    let mut options = inferno::flamegraph::Options::default();
    options.title = "stutter latency attribution flamegraph".to_string();

    let input = lines.join("\n") + "\n";
    let mut output = Vec::new();

    inferno::flamegraph::from_reader(&mut options, input.as_bytes(), &mut output)
        .context("failed to render latency flamegraph SVG")?;

    String::from_utf8(output).context("flamegraph renderer returned non-UTF8 SVG")
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::process_tree::TaskClass;

    #[test]
    fn sanitize_folded_label_replaces_separators_and_whitespace() {
        assert_eq!(sanitize_folded_label("Render;Thread"), "Render_Thread");
        assert_eq!(sanitize_folded_label("Game Thread"), "Game_Thread");
        assert_eq!(sanitize_folded_label(" \t "), "unknown");
    }

    #[test]
    fn folded_latency_stacks_aggregate_by_pseudo_stack() {
        let spikes = vec![
            test_spike("Game", "RenderThread", 7, 1_000_000),
            test_spike("Game", "RenderThread", 7, 2_000_000),
        ];

        let lines = build_folded_latency_stacks(&spikes);

        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("Game;RenderThread;cpu7 3000000"));
    }

    #[test]
    fn render_latency_flamegraph_svg_contains_svg_tag() {
        let lines = vec!["Game;RenderThread;cpu7 1000000".to_string()];
        let svg = render_flamegraph_svg_from_folded_lines(&lines).unwrap();

        assert!(svg.contains("<svg"));
    }

    fn test_spike(process: &str, comm: &str, cpu: u32, latency_ns: u64) -> SpikeEvent {
        SpikeEvent {
            process_comm: Arc::from(process),
            comm: comm.to_string(),
            cpu,
            latency_ns,
            class: TaskClass::Game,
            task: 123,
            active: true,
            ..Default::default()
        }
    }
}
