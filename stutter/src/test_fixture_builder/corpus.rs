//! Public corpus-writing entry points and fixture file serialization.

use super::{fixtures::*, metadata::fixture_metadata_for, *};

const OPTIONAL_ARTIFACT_FILES: &[&str] = &[
    "spike_events.json",
    "interval.json",
    "tree_events.json",
    "irq_events.json",
    "gpu_samples.json",
    "frame_correlation.json",
    "frame_events.json",
    "migration_events.json",
    "cpu_freq_samples.json",
    "io_events.json",
    "scx_events.json",
    "focus_events.json",
    "foreground_events.json",
    "runtime_slices.json",
];

pub(crate) fn write_validation_corpus(root: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(root)
        .with_context(|| format!("failed to create fixture root {}", root.display()))?;

    for deprecated in [
        "real_world_game_scheduler_delay",
        "real_world_compositor_scheduler_delay",
        "real_world_irq_overlap",
        "real_world_block_io_stall",
        "real_world_gpu_bound_clean_cpu",
    ] {
        remove_fixture_dir(root, deprecated)?;
    }

    write_fixture(root, "clean_run", clean_run_fixture())?;
    write_fixture(root, "cpu_pressure", cpu_pressure_fixture())?;
    write_fixture(root, "block_io_stall", block_io_stall_fixture())?;
    write_fixture(root, "irq_heavy", irq_heavy_fixture())?;
    write_fixture(root, "gpu_bound_clean_cpu", gpu_bound_clean_cpu_fixture())?;
    write_fixture(
        root,
        "truncated_drop_counters",
        truncated_drop_counters_fixture(),
    )?;
    write_fixture(
        root,
        "reused_tid_no_contamination",
        reused_tid_no_contamination_fixture(),
    )?;
    write_fixture(root, "old_schema_warning", old_schema_warning_fixture())?;

    write_fixture(
        root,
        "game_thread_scheduler_delay",
        game_thread_scheduler_delay_fixture(),
    )?;
    write_fixture(
        root,
        "compositor_scheduler_delay",
        compositor_scheduler_delay_fixture(),
    )?;
    write_fixture(root, "foreground_window", foreground_window_fixture())?;
    write_fixture(
        root,
        "community_rules_classification",
        community_rules_classification_fixture(),
    )?;
    write_fixture(root, "real_clean_baseline", real_clean_baseline_fixture())?;
    write_fixture(
        root,
        "real_game_thread_scheduler_delay",
        real_game_thread_scheduler_delay_fixture(),
    )?;
    write_fixture(
        root,
        "real_compositor_scheduler_delay",
        real_compositor_scheduler_delay_fixture(),
    )?;
    write_fixture(root, "real_irq_overlap", real_irq_overlap_fixture())?;
    write_fixture(
        root,
        "real_gpu_bound_looking",
        real_gpu_bound_looking_fixture(),
    )?;
    write_fixture(
        root,
        "real_block_io_overlap",
        real_block_io_overlap_fixture(),
    )?;
    write_fixture(
        root,
        "real_truncated_low_quality",
        real_truncated_low_quality_fixture(),
    )?;
    write_fixture(
        root,
        "real_foreground_window",
        real_foreground_window_fixture(),
    )?;
    write_fixture(
        root,
        "real_community_rules_classification",
        real_community_rules_classification_fixture(),
    )?;

    Ok(())
}

pub(crate) fn write_public_examples_v21(root: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(root)
        .with_context(|| format!("failed to create public example root {}", root.display()))?;

    for deprecated in [
        "real_world_game_scheduler_delay",
        "real_world_compositor_scheduler_delay",
        "real_world_gpu_bound_clean_cpu",
    ] {
        remove_fixture_dir(root, deprecated)?;
    }

    write_fixture(root, "clean_baseline", public_clean_baseline_fixture())?;
    write_fixture(
        root,
        "game_thread_scheduler_delay",
        renamed_fixture(
            "game_thread_scheduler_delay",
            public_game_thread_scheduler_delay_fixture(),
        ),
    )?;
    write_fixture(
        root,
        "low_quality_truncated",
        public_low_quality_truncated_fixture(),
    )?;
    write_public_examples_readme_v21(root)?;

    Ok(())
}

pub(crate) fn write_autotune_replay_corpus(root: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(root)
        .with_context(|| format!("failed to create replay fixture root {}", root.display()))?;

    write_fixture(
        root,
        "game_scheduler_pressure",
        game_scheduler_pressure_fixture(),
    )?;
    write_fixture(root, "gpu_bound", gpu_bound_clean_cpu_fixture())?;
    write_fixture(root, "low_quality", truncated_drop_counters_fixture())?;

    Ok(())
}

fn remove_fixture_dir(root: &Path, name: &str) -> anyhow::Result<()> {
    let dir = root.join(name);
    if dir.exists() {
        fs::remove_dir_all(&dir).with_context(|| {
            format!("failed to remove deprecated fixture dir {}", dir.display())
        })?;
    }
    Ok(())
}

pub(crate) fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("runs")
        .join(name)
}

fn write_fixture(
    root: &Path,
    name: &str,
    (session, artifacts): (SessionFile, FixtureArtifacts),
) -> anyhow::Result<()> {
    let dir = root.join(name);
    if dir.exists() {
        fs::remove_dir_all(&dir)
            .with_context(|| format!("failed to remove fixture dir {}", dir.display()))?;
    }
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create fixture dir {}", dir.display()))?;

    let fixture_metadata = fixture_metadata_for(name, &artifacts);

    write_toml_pretty(dir.join("fixture.toml"), &fixture_metadata)?;
    write_json_pretty(dir.join("session.json"), &session)?;
    write_json_pretty(
        dir.join("metadata.json"),
        &MetadataFile {
            core: session.core.clone(),
        },
    )?;

    for file in OPTIONAL_ARTIFACT_FILES {
        write_ndjson_values::<serde_json::Value>(dir.join(file), &[])?;
    }

    write_ndjson_values(dir.join("spike_events.json"), &artifacts.spikes)?;
    write_ndjson_values(dir.join("interval.json"), &artifacts.intervals)?;
    write_ndjson_values(dir.join("irq_events.json"), &artifacts.irq_events)?;
    write_ndjson_values(dir.join("gpu_samples.json"), &artifacts.gpu_samples)?;
    write_ndjson_values(dir.join("frame_correlation.json"), &artifacts.frame_events)?;
    write_ndjson_values(dir.join("io_events.json"), &artifacts.block_io_events)?;
    write_ndjson_values(
        dir.join("foreground_events.json"),
        &artifacts.foreground_events,
    )?;

    Ok(())
}

fn write_public_examples_readme_v21(root: &Path) -> anyhow::Result<()> {
    let readme = r#"# stutter v21 public artifact examples

This directory intentionally contains only small, representative sanitized examples.

## Examples

| Directory                      | Purpose                                        |
| ------------------------------ | ---------------------------------------------- |
| `clean_baseline/`              | Quiet baseline run with no strong diagnosis.   |
| `game_thread_scheduler_delay/` | Game-thread scheduler-delay diagnosis example. |
| `low_quality_truncated/`       | Low-quality/truncated data-quality example.    |

The larger regression corpus lives under:

```text
stutter/tests/fixtures/runs/
```

Do not duplicate every large validation fixture here unless repository size stays reasonable.
"#;

    fs::write(root.join("README.md"), readme).with_context(|| {
        format!(
            "failed to write public examples README under {}",
            root.display()
        )
    })
}

fn write_toml_pretty<T: serde::Serialize>(path: impl AsRef<Path>, value: &T) -> anyhow::Result<()> {
    let path = path.as_ref();
    let text = toml::to_string_pretty(value)
        .with_context(|| format!("failed to serialize TOML fixture {}", path.display()))?;
    fs::write(path, text)
        .with_context(|| format!("failed to write TOML fixture {}", path.display()))?;
    Ok(())
}

fn write_json_pretty<T: serde::Serialize>(path: impl AsRef<Path>, value: &T) -> anyhow::Result<()> {
    let path = path.as_ref();
    let file = fs::File::create(path)
        .with_context(|| format!("failed to create JSON fixture {}", path.display()))?;
    serde_json::to_writer_pretty(file, value)
        .with_context(|| format!("failed to write JSON fixture {}", path.display()))?;
    Ok(())
}

fn write_ndjson_values<T: serde::Serialize>(
    path: impl AsRef<Path>,
    values: &[T],
) -> anyhow::Result<()> {
    let path = path.as_ref();
    let mut file = fs::File::create(path)
        .with_context(|| format!("failed to create NDJSON fixture {}", path.display()))?;
    for value in values {
        serde_json::to_writer(&mut file, value)
            .with_context(|| format!("failed to write NDJSON fixture {}", path.display()))?;
        file.write_all(b"\n")
            .with_context(|| format!("failed to write NDJSON fixture {}", path.display()))?;
    }
    Ok(())
}
