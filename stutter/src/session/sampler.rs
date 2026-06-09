//! Sampler setup for monitor sessions.

use crate::{config::model::MonitorConfig, runtime_slices::RuntimeSliceSampler};

pub(crate) struct SamplerRuntime {
    pub(crate) cpu_perf_sampler: Option<crate::perf_counters::CpuPerfSampler>,
    pub(crate) runtime_slice_sampler: Option<RuntimeSliceSampler>,
}

impl SamplerRuntime {
    pub(crate) fn begin(config: &MonitorConfig) -> Self {
        let cpu_perf_sampler = if config.probes.cpu_perf {
            Some(crate::perf_counters::CpuPerfSampler::new(
                crate::perf_counters::CpuPerfConfig {
                    include_kernel: config.cpu_perf.include_kernel,
                    max_tasks: config.cpu_perf.max_tasks,
                    collect_cache_refs: config.cpu_perf.collect_cache_refs,
                },
            ))
        } else {
            None
        };

        let runtime_slice_sampler = config.probes.runtime_slices.then(RuntimeSliceSampler::new);

        Self {
            cpu_perf_sampler,
            runtime_slice_sampler,
        }
    }
}
