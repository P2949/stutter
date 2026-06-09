pub struct ProbeRuntime {
    pub block_io_correlation_basis: String,
    pub block_io_correlation_confidence: String,
    pub native_cgroup_filter: crate::ebpf_loader::NativeCgroupFilterStatus,
    pub cpu_perf_sampler: Option<crate::perf_counters::CpuPerfSampler>,
    pub runtime_slice_sampler: Option<crate::runtime_slices::RuntimeSliceSampler>,
    pub psi_reader: crate::psi::PsiReader,
    pub scx_tracker: crate::scx::ScxTracker,
}

impl ProbeRuntime {
    pub fn new(
        block_io_correlation_basis: String,
        block_io_correlation_confidence: String,
        native_cgroup_filter: crate::ebpf_loader::NativeCgroupFilterStatus,
        cpu_perf_sampler: Option<crate::perf_counters::CpuPerfSampler>,
        runtime_slice_sampler: Option<crate::runtime_slices::RuntimeSliceSampler>,
    ) -> Self {
        let mut scx_tracker = crate::scx::ScxTracker::default();
        scx_tracker.sample(0);

        Self {
            block_io_correlation_basis,
            block_io_correlation_confidence,
            native_cgroup_filter,
            cpu_perf_sampler,
            runtime_slice_sampler,
            psi_reader: crate::psi::PsiReader::new(),
            scx_tracker,
        }
    }
}
