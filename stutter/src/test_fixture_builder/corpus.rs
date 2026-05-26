//! Public corpus-writing entry points and fixture file serialization.

use super::{fixtures::*, metadata::fixture_metadata_for, *};
use crate::artifacts::{ArtifactKind, artifact_alias_paths, artifact_path};

const OPTIONAL_ARTIFACT_KINDS: &[ArtifactKind] = &[
    ArtifactKind::SpikeEvents,
    ArtifactKind::Interval,
    ArtifactKind::TreeEvents,
    ArtifactKind::IrqEvents,
    ArtifactKind::GpuSamples,
    ArtifactKind::FrameEvents,
    ArtifactKind::MigrationEvents,
    ArtifactKind::CpuFreqSamples,
    ArtifactKind::BlockIoEvents,
    ArtifactKind::ScxEvents,
    ArtifactKind::FocusEvents,
    ArtifactKind::ForegroundEvents,
    ArtifactKind::RuntimeSlices,
    ArtifactKind::KmsFlipEvents,
    ArtifactKind::DrmFenceEvents,
    ArtifactKind::WaylandPresentationEvents,
    ArtifactKind::DmaBufEvents,
    ArtifactKind::GpuEngineSamples,
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
    write_fixture(root, "direct_gpu_clean", direct_gpu_clean_fixture())?;
    write_fixture(
        root,
        "uhd630_cross_gpu_fence_wait",
        uhd630_cross_gpu_fence_wait_fixture(),
    )?;
    write_fixture(
        root,
        "uhd630_composited_blitter",
        uhd630_composited_blitter_fixture(),
    )?;
    write_fixture(root, "uhd630_kms_delay", uhd630_kms_delay_fixture())?;
    write_fixture(
        root,
        "wayland_zero_copy_good",
        wayland_zero_copy_good_fixture(),
    )?;
    write_fixture(
        root,
        "dmabuf_modifier_mismatch",
        dmabuf_modifier_mismatch_fixture(),
    )?;
    write_fixture(
        root,
        "missing_evidence_unknown",
        missing_evidence_unknown_fixture(),
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

pub(crate) fn write_public_examples_v22(root: &Path) -> anyhow::Result<()> {
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
    write_public_examples_readme_v22(root)?;

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
    write_json_pretty(artifact_path(&dir, ArtifactKind::Session), &session)?;
    write_json_pretty(
        artifact_path(&dir, ArtifactKind::Metadata),
        &MetadataFile {
            core: session.core.clone(),
        },
    )?;
    write_json_pretty(
        artifact_path(&dir, ArtifactKind::DisplayTopology),
        &artifacts.display_topology.clone().unwrap_or_default(),
    )?;

    for kind in OPTIONAL_ARTIFACT_KINDS {
        write_ndjson_values::<serde_json::Value>(artifact_path(&dir, *kind), &[])?;
    }
    for path in artifact_alias_paths(&dir, ArtifactKind::FrameEvents) {
        write_ndjson_values::<serde_json::Value>(path, &[])?;
    }

    write_ndjson_values(
        artifact_path(&dir, ArtifactKind::SpikeEvents),
        &artifacts.spikes,
    )?;
    write_ndjson_values(
        artifact_path(&dir, ArtifactKind::Interval),
        &artifacts.intervals,
    )?;
    write_ndjson_values(
        artifact_path(&dir, ArtifactKind::IrqEvents),
        &artifacts.irq_events,
    )?;
    write_ndjson_values(
        artifact_path(&dir, ArtifactKind::GpuSamples),
        &artifacts.gpu_samples,
    )?;
    write_ndjson_values(
        artifact_path(&dir, ArtifactKind::FrameEvents),
        &artifacts.frame_events,
    )?;
    write_ndjson_values(
        artifact_path(&dir, ArtifactKind::BlockIoEvents),
        &artifacts.block_io_events,
    )?;
    write_ndjson_values(
        artifact_path(&dir, ArtifactKind::ForegroundEvents),
        &artifacts.foreground_events,
    )?;
    write_ndjson_values(
        artifact_path(&dir, ArtifactKind::KmsFlipEvents),
        &artifacts.kms_flip_events,
    )?;
    write_ndjson_values(
        artifact_path(&dir, ArtifactKind::DrmFenceEvents),
        &artifacts.drm_fence_events,
    )?;
    write_ndjson_values(
        artifact_path(&dir, ArtifactKind::WaylandPresentationEvents),
        &artifacts.wayland_presentation_events,
    )?;
    write_ndjson_values(
        artifact_path(&dir, ArtifactKind::DmaBufEvents),
        &artifacts.dmabuf_events,
    )?;
    write_ndjson_values(
        artifact_path(&dir, ArtifactKind::GpuEngineSamples),
        &artifacts.gpu_engine_samples,
    )?;

    Ok(())
}

fn write_public_examples_readme_v22(root: &Path) -> anyhow::Result<()> {
    let readme = r#"# stutter v22 public artifact examples

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
