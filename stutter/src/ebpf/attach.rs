use aya::{
    Ebpf,
    programs::{PerfEvent, TracePoint},
    util::online_cpus,
};

use crate::{
    drm_fence_tracepoints::{DrmFenceTracepointDiscovery, DrmFenceTracepointFormat},
    drm_tracepoints::{
        DrmTracepointFormat, KmsTracepointAvailability, KmsTracepointProvider,
        selected_request_format,
    },
    ebpf::tracepoints::drm_fence::DrmFenceTracepointOffsets,
    error::EbpfError,
    probe_activation::ProbeActivationPlan,
    probe_registry::ProbeKey,
};

#[derive(Debug, thiserror::Error)]
#[error(
    "failed to attach eBPF program {program_name} to tracepoint {category}/{tracepoint_name}: {source:#}"
)]
pub(crate) struct TracepointAttachError {
    program_name: &'static str,
    category: String,
    tracepoint_name: String,
    #[source]
    source: anyhow::Error,
}

impl TracepointAttachError {
    pub(crate) fn new(
        program_name: &'static str,
        category: &str,
        tracepoint_name: &str,
        source: anyhow::Error,
    ) -> Self {
        Self {
            program_name,
            category: category.to_owned(),
            tracepoint_name: tracepoint_name.to_owned(),
            source,
        }
    }

    #[cfg(test)]
    pub(crate) fn program_name(&self) -> &'static str {
        self.program_name
    }

    #[cfg(test)]
    pub(crate) fn category(&self) -> &str {
        &self.category
    }

    #[cfg(test)]
    pub(crate) fn tracepoint_name(&self) -> &str {
        &self.tracepoint_name
    }

    pub(crate) fn source(&self) -> &anyhow::Error {
        &self.source
    }

    fn into_ebpf_error(self) -> EbpfError {
        EbpfError::TracepointAttach {
            program: self.program_name,
            category: self.category,
            tracepoint: self.tracepoint_name,
            source: self.source,
        }
    }
}

impl From<TracepointAttachError> for EbpfError {
    fn from(error: TracepointAttachError) -> Self {
        error.into_ebpf_error()
    }
}

pub(crate) trait AttachOps {
    fn attach_tracepoint(
        &mut self,
        program_name: &'static str,
        category: &str,
        tracepoint_name: &str,
    ) -> Result<(), TracepointAttachError>;

    fn attach_perf_event(
        &mut self,
        program_name: &'static str,
        probe: FaultPerfProbe,
    ) -> anyhow::Result<()>;
}

pub(crate) struct AyaAttachOps<'a> {
    ebpf: &'a mut Ebpf,
}

impl<'a> AyaAttachOps<'a> {
    pub(crate) fn new(ebpf: &'a mut Ebpf) -> Self {
        Self { ebpf }
    }
}

impl AttachOps for AyaAttachOps<'_> {
    fn attach_tracepoint(
        &mut self,
        program_name: &'static str,
        category: &str,
        tracepoint_name: &str,
    ) -> Result<(), TracepointAttachError> {
        attach_tracepoint(self.ebpf, program_name, category, tracepoint_name)
    }

    fn attach_perf_event(
        &mut self,
        program_name: &'static str,
        probe: FaultPerfProbe,
    ) -> anyhow::Result<()> {
        attach_software_perf_event(self.ebpf, program_name, probe)
    }
}

pub(crate) fn attach_tracepoint(
    ebpf: &mut Ebpf,
    program_name: &'static str,
    category: &str,
    tracepoint_name: &str,
) -> Result<(), TracepointAttachError> {
    let attach_result = (|| -> anyhow::Result<()> {
        let program: &mut TracePoint = ebpf
            .program_mut(program_name)
            .ok_or_else(|| anyhow::anyhow!("{program_name} program not found"))?
            .try_into()?;

        program.load()?;
        program.attach(category, tracepoint_name)?;
        Ok(())
    })();

    attach_result.map_err(|source| {
        TracepointAttachError::new(program_name, category, tracepoint_name, source)
    })
}

pub(crate) fn attach_kms_tracepoints(
    ops: &mut impl AttachOps,
    activation_plan: &mut ProbeActivationPlan,
    kms: &KmsTracepointAvailability,
) {
    let (request_program, done_program, vblank_program) = match kms.provider {
        KmsTracepointProvider::GenericDrm => (
            "drm_flip_request",
            "drm_flip_done",
            Some("drm_vblank_event"),
        ),
        KmsTracepointProvider::I915 => ("i915_flip_request", "i915_flip_done", None),
        KmsTracepointProvider::Amdgpu => (
            "amdgpu_flip_request",
            "amdgpu_flip_done",
            Some("amdgpu_vblank_event"),
        ),
        KmsTracepointProvider::Mixed | KmsTracepointProvider::Unavailable => return,
    };

    if let Some(request) = selected_request_format(kms) {
        attach_optional_kms_tracepoint(ops, activation_plan, request_program, request);
    }
    if let Some(done) = kms.pageflip_done.as_ref() {
        attach_optional_kms_tracepoint(ops, activation_plan, done_program, done);
    }
    if let (Some(program), Some(vblank)) = (vblank_program, kms.vblank_event.as_ref()) {
        attach_optional_kms_tracepoint(ops, activation_plan, program, vblank);
    }
}

fn attach_optional_kms_tracepoint(
    ops: &mut impl AttachOps,
    activation_plan: &mut ProbeActivationPlan,
    program_name: &'static str,
    tracepoint: &DrmTracepointFormat,
) {
    if let Err(err) = ops.attach_tracepoint(program_name, &tracepoint.category, &tracepoint.name) {
        activation_plan.push_tracepoint_attach_warning(
            ProbeKey::KmsPageflipTiming,
            program_name,
            &tracepoint.category,
            &tracepoint.name,
            err.source(),
        );
        log::warn!(
            "optional_probe_attach_failed key={:?} program={} tracepoint={} err={:#}",
            ProbeKey::KmsPageflipTiming,
            program_name,
            tracepoint.ref_name(),
            err.source()
        );
    }
}

pub(crate) fn attach_drm_fence_tracepoints(
    ops: &mut impl AttachOps,
    activation_plan: &mut ProbeActivationPlan,
    discovery: &DrmFenceTracepointDiscovery,
    offsets: DrmFenceTracepointOffsets,
) {
    if offsets.has_wait_interval {
        if let Some(start) = discovery.selected_wait_start() {
            attach_optional_drm_fence_tracepoint(
                ops,
                activation_plan,
                "drm_fence_wait_start",
                start,
            );
        }
        if let Some(done) = discovery.selected_wait_done() {
            attach_optional_drm_fence_tracepoint(ops, activation_plan, "drm_fence_wait_done", done);
        }
    }
    if offsets.has_signal
        && let Some(signal) = discovery.selected_signal()
    {
        attach_optional_drm_fence_tracepoint(ops, activation_plan, "drm_fence_signal", signal);
    }
}

fn attach_optional_drm_fence_tracepoint(
    ops: &mut impl AttachOps,
    activation_plan: &mut ProbeActivationPlan,
    program_name: &'static str,
    tracepoint: &DrmFenceTracepointFormat,
) {
    if let Err(err) = ops.attach_tracepoint(program_name, &tracepoint.category, &tracepoint.name) {
        activation_plan.push_tracepoint_attach_warning(
            ProbeKey::DrmFenceLatency,
            program_name,
            &tracepoint.category,
            &tracepoint.name,
            err.source(),
        );
        log::warn!(
            "optional_probe_attach_failed key={:?} program={} tracepoint={}/{} err={:#}",
            ProbeKey::DrmFenceLatency,
            program_name,
            tracepoint.category,
            tracepoint.name,
            err.source()
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FaultPerfProbe {
    Minor,
    Major,
}

impl FaultPerfProbe {
    fn software_event(self) -> aya::programs::perf_event::SoftwareEvent {
        match self {
            Self::Minor => aya::programs::perf_event::SoftwareEvent::PageFaultsMin,
            Self::Major => aya::programs::perf_event::SoftwareEvent::PageFaultsMaj,
        }
    }
}

pub(crate) fn attach_software_perf_event(
    ebpf: &mut Ebpf,
    program_name: &str,
    probe: FaultPerfProbe,
) -> anyhow::Result<()> {
    let program: &mut PerfEvent = ebpf
        .program_mut(program_name)
        .ok_or_else(|| anyhow::anyhow!("{program_name} program not found"))?
        .try_into()?;

    program.load()?;

    for cpu in online_cpus().map_err(|e| anyhow::anyhow!("{}: {}", e.0, e.1))? {
        let sw_event = probe.software_event();
        program.attach(
            aya::programs::perf_event::PerfEventConfig::Software(sw_event),
            aya::programs::perf_event::PerfEventScope::AllProcessesOneCpu { cpu },
            aya::programs::perf_event::SamplePolicy::Period(1),
            true, // inherit
        )?;
    }

    Ok(())
}
