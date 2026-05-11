pub struct ProbeRuntime {
    pub loaded: crate::ebpf_loader::LoadedEbpf,
    pub block_io_correlation_basis: String,
    pub block_io_correlation_confidence: String,
    pub cpu_perf_sampler: Option<crate::perf_counters::CpuPerfSampler>,
    pub runtime_slice_sampler: Option<crate::runtime_slices::RuntimeSliceSampler>,
    pub psi_reader: crate::psi::PsiReader,
    pub scx_tracker: crate::scx::ScxTracker,
}

impl ProbeRuntime {
    pub fn new(
        loaded: crate::ebpf_loader::LoadedEbpf,
        block_io_correlation_basis: String,
        block_io_correlation_confidence: String,
        cpu_perf_sampler: Option<crate::perf_counters::CpuPerfSampler>,
        runtime_slice_sampler: Option<crate::runtime_slices::RuntimeSliceSampler>,
    ) -> Self {
        let mut scx_tracker = crate::scx::ScxTracker::default();
        scx_tracker.sample(0);

        Self {
            loaded,
            block_io_correlation_basis,
            block_io_correlation_confidence,
            cpu_perf_sampler,
            runtime_slice_sampler,
            psi_reader: crate::psi::PsiReader::new(),
            scx_tracker,
        }
    }

    pub fn activation_plan(&self) -> &crate::probe_activation::ProbeActivationPlan {
        &self.loaded.activation_plan
    }
}
