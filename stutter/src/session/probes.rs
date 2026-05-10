pub struct ProbeRuntime {
    pub loaded: crate::ebpf_loader::LoadedEbpf,
    pub block_io_correlation_basis: String,
    pub cpu_perf_sampler: Option<crate::perf_counters::CpuPerfSampler>,
    pub runtime_slice_sampler: Option<crate::runtime_slices::RuntimeSliceSampler>,
    pub psi_reader: crate::psi::PsiReader,
    pub scx_tracker: crate::scx::ScxTracker,
}

impl ProbeRuntime {
    pub fn new(
        loaded: crate::ebpf_loader::LoadedEbpf,
        block_io_correlation_basis: String,
        cpu_perf_sampler: Option<crate::perf_counters::CpuPerfSampler>,
        runtime_slice_sampler: Option<crate::runtime_slices::RuntimeSliceSampler>,
    ) -> Self {
        let mut scx_tracker = crate::scx::ScxTracker::default();
        scx_tracker.sample(0);

        Self {
            loaded,
            block_io_correlation_basis,
            cpu_perf_sampler,
            runtime_slice_sampler,
            psi_reader: crate::psi::PsiReader::new(),
            scx_tracker,
        }
    }

    pub fn from_config_parts(
        loaded: crate::ebpf_loader::LoadedEbpf,
        block_io_correlation_basis: String,
        cpu_perf_sampler: Option<crate::perf_counters::CpuPerfSampler>,
        runtime_slice_sampler: Option<crate::runtime_slices::RuntimeSliceSampler>,
    ) -> Self {
        Self::new(
            loaded,
            block_io_correlation_basis,
            cpu_perf_sampler,
            runtime_slice_sampler,
        )
    }
}
