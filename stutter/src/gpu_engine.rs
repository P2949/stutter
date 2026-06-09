//! GPU engine activity sampling and report summaries.
//!
//! Owns hwmon-derived multi-GPU engine samples and derived engine activity summaries. It does
//! not own generic GPU telemetry, DRM topology detection, or low-level PMU/fdinfo accounting.

use std::{collections::BTreeSet, path::PathBuf};

use crate::{
    config::model::MonitorConfig,
    display_topology::{DisplayTopologySnapshot, DrmDeviceInfo},
    hwmon::HwmonReader,
    recorder::{GpuEngineSample, GpuSample},
};

pub(crate) trait EngineSampler {
    fn sample(&mut self, elapsed_ms: u64) -> Vec<GpuEngineSample>;
}

#[derive(Debug)]
pub(crate) struct MultiGpuHwmonReader {
    readers: Vec<HwmonEngineReader>,
}

#[derive(Debug)]
pub(crate) struct HwmonEngineReader {
    reader: HwmonReader,
    drm_card: Option<String>,
    render_node: Option<String>,
    driver: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct EngineReaderCandidate {
    drm_card: Option<String>,
    render_node: Option<String>,
    driver: Option<String>,
}

impl MultiGpuHwmonReader {
    pub(crate) fn discover(config: &MonitorConfig) -> Option<Self> {
        if !config.probes.gpu_engine_sampling {
            return None;
        }

        if config.hwmon.root.is_some() || config.hwmon.drm_card.is_some() {
            return HwmonReader::discover_with_options(
                config.hwmon.root.as_deref(),
                config.hwmon.drm_card.as_deref(),
                config.hwmon.render_node.as_deref(),
            )
            .map(|reader| {
                Self::from_readers(vec![HwmonEngineReader {
                    reader,
                    drm_card: config.hwmon.drm_card.clone(),
                    render_node: config
                        .hwmon
                        .render_node
                        .as_ref()
                        .and_then(|node| node.file_name())
                        .and_then(|name| name.to_str())
                        .map(str::to_owned),
                    driver: None,
                }])
            });
        }

        let topology = crate::display_topology::probe_display_topology();
        let mut candidates = candidates_from_topology(&topology);
        if candidates.is_empty()
            && let Some(render_node) = config.hwmon.render_node.as_ref()
        {
            candidates.insert(EngineReaderCandidate {
                drm_card: None,
                render_node: render_node
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_owned),
                driver: None,
            });
        }

        let readers = candidates
            .into_iter()
            .filter_map(|candidate| {
                let render_node_path = candidate.render_node.as_deref().map(PathBuf::from);
                HwmonReader::discover_with_options(
                    None,
                    candidate.drm_card.as_deref(),
                    render_node_path.as_deref(),
                )
                .map(|reader| HwmonEngineReader {
                    reader,
                    drm_card: candidate.drm_card,
                    render_node: candidate.render_node,
                    driver: candidate.driver,
                })
            })
            .collect::<Vec<_>>();

        if readers.is_empty() {
            HwmonReader::discover_with_options(None, None, config.hwmon.render_node.as_deref()).map(
                |reader| {
                    Self::from_readers(vec![HwmonEngineReader {
                        reader,
                        drm_card: None,
                        render_node: config
                            .hwmon
                            .render_node
                            .as_ref()
                            .and_then(|node| node.file_name())
                            .and_then(|name| name.to_str())
                            .map(str::to_owned),
                        driver: None,
                    }])
                },
            )
        } else {
            Some(Self::from_readers(readers))
        }
    }

    pub(crate) fn from_readers(readers: Vec<HwmonEngineReader>) -> Self {
        Self { readers }
    }
}

impl EngineSampler for MultiGpuHwmonReader {
    fn sample(&mut self, elapsed_ms: u64) -> Vec<GpuEngineSample> {
        self.readers
            .iter_mut()
            .filter_map(|reader| reader.sample(elapsed_ms))
            .collect()
    }
}

impl HwmonEngineReader {
    #[cfg(test)]
    pub(crate) fn new(
        reader: HwmonReader,
        drm_card: Option<String>,
        render_node: Option<String>,
        driver: Option<String>,
    ) -> Self {
        Self {
            reader,
            drm_card,
            render_node,
            driver,
        }
    }

    fn sample(&mut self, elapsed_ms: u64) -> Option<GpuEngineSample> {
        let sample = self.reader.sample(elapsed_ms);
        engine_sample_from_hwmon(sample, self)
    }
}

fn engine_sample_from_hwmon(
    sample: GpuSample,
    reader: &HwmonEngineReader,
) -> Option<GpuEngineSample> {
    let busy = sample.gpu_busy_percent.map(|value| value as f64)?;
    let driver = reader.driver.clone();
    let engine = default_engine_for_driver(driver.as_deref()).to_owned();
    Some(GpuEngineSample {
        elapsed_ms: sample.elapsed_ms,
        drm_card: sample.drm_card.or_else(|| reader.drm_card.clone()),
        render_node: sample.render_node.or_else(|| reader.render_node.clone()),
        driver,
        engine,
        busy_percent: Some(busy),
        client_pid: None,
        client_comm: None,
        source: "hwmon".to_owned(),
        confidence: "medium".to_owned(),
    })
}

fn candidates_from_topology(topology: &DisplayTopologySnapshot) -> BTreeSet<EngineReaderCandidate> {
    let mut candidates = BTreeSet::new();
    if let Some(guess) = topology.guessed_path.as_ref() {
        push_guess_candidate(&mut candidates, topology, guess.render_card.as_deref());
        push_guess_candidate(&mut candidates, topology, guess.scanout_card.as_deref());
    }
    candidates
}

fn push_guess_candidate(
    candidates: &mut BTreeSet<EngineReaderCandidate>,
    topology: &DisplayTopologySnapshot,
    card: Option<&str>,
) {
    let Some(card) = card else {
        return;
    };
    let device = topology_device(topology, card);
    candidates.insert(EngineReaderCandidate {
        drm_card: Some(card.to_owned()),
        render_node: device.and_then(|device| device.render_node.clone()),
        driver: device.and_then(|device| device.driver.clone()),
    });
}

fn topology_device<'a>(
    topology: &'a DisplayTopologySnapshot,
    card: &str,
) -> Option<&'a DrmDeviceInfo> {
    topology
        .drm_devices
        .iter()
        .find(|device| device.card == card)
}

fn default_engine_for_driver(driver: Option<&str>) -> &'static str {
    match driver {
        Some("amdgpu") => "gfx",
        Some("i915") | Some("xe") => "render",
        _ => "gpu",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hwmon_reader_emits_engine_sample_with_identity() {
        let root = temp_dir("gpu-engine-hwmon");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("gpu_busy_percent"), "42\n").unwrap();
        let reader = HwmonReader::discover_with_options(Some(&root), Some("card1"), None).unwrap();
        let mut sampler = MultiGpuHwmonReader::from_readers(vec![HwmonEngineReader::new(
            reader,
            Some("card1".to_owned()),
            None,
            Some("amdgpu".to_owned()),
        )]);

        let samples = sampler.sample(123);

        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].elapsed_ms, 123);
        assert_eq!(samples[0].drm_card.as_deref(), Some("card1"));
        assert_eq!(samples[0].driver.as_deref(), Some("amdgpu"));
        assert_eq!(samples[0].engine, "gfx");
        assert_eq!(samples[0].busy_percent, Some(42.0));

        std::fs::remove_dir_all(root).ok();
    }

    fn temp_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "stutter-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&path);
        path
    }
}
