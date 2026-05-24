//! eBPF map initialization seam.
//!
//! This keeps real Aya map extraction in production while allowing non-root
//! tests to exercise missing-map, bad-map-type, and async-fd failure behavior
//! without constructing a real `aya::Ebpf`.

use anyhow::Context;
use aya::{
    Ebpf,
    maps::{HashMap as AyaHashMap, MapData, PerCpuArray, RingBuf},
};
use tokio::io::unix::AsyncFd;

const TARGET_PIDS_MAP: &str = "TARGET_PIDS";
const TARGET_IRQS_MAP: &str = "TARGET_IRQS";
const DROP_COUNTERS_MAP: &str = "DROP_COUNTERS";
const EVENTS_MAP: &str = "EVENTS";
const PREV_FAULTS_MAP: &str = "PREV_FAULTS";

pub(crate) fn missing_map_context(map: &'static str) -> String {
    format!("eBPF load failed: {map} map not found")
}

pub(crate) fn map_init_context(map: &'static str) -> String {
    format!("eBPF load failed: {map} map init")
}

pub(crate) trait MapInitOps {
    type TargetPidMap;
    type TargetIrqMap;
    type DropCounters;
    type Events;
    type PrevFaultsMap;

    fn target_pid_map(&mut self) -> anyhow::Result<Self::TargetPidMap>;

    fn target_irq_map(&mut self) -> anyhow::Result<Option<Self::TargetIrqMap>>;

    fn drop_counters(&mut self) -> anyhow::Result<Self::DropCounters>;

    fn events(&mut self) -> anyhow::Result<Self::Events>;

    fn prev_faults_map(&mut self) -> anyhow::Result<Option<Self::PrevFaultsMap>>;
}

#[derive(Debug)]
pub(crate) struct InitializedEbpfMaps<O: MapInitOps> {
    pub(crate) target_pid_map: O::TargetPidMap,
    pub(crate) target_irq_map: Option<O::TargetIrqMap>,
    pub(crate) drop_counters: O::DropCounters,
    pub(crate) events: O::Events,
    pub(crate) prev_faults_map: Option<O::PrevFaultsMap>,
}

pub(crate) fn initialize_ebpf_maps<O: MapInitOps>(
    ops: &mut O,
) -> anyhow::Result<InitializedEbpfMaps<O>> {
    let target_pid_map = ops.target_pid_map()?;
    let target_irq_map = ops.target_irq_map()?;
    let drop_counters = ops.drop_counters()?;
    let events = ops.events()?;
    let prev_faults_map = ops.prev_faults_map()?;

    Ok(InitializedEbpfMaps {
        target_pid_map,
        target_irq_map,
        drop_counters,
        events,
        prev_faults_map,
    })
}

pub(crate) struct AyaMapInitOps<'a> {
    ebpf: &'a mut Ebpf,
}

impl<'a> AyaMapInitOps<'a> {
    pub(crate) fn new(ebpf: &'a mut Ebpf) -> Self {
        Self { ebpf }
    }

    fn required_hash_u32_u8(
        &mut self,
        map_name: &'static str,
    ) -> anyhow::Result<AyaHashMap<MapData, u32, u8>> {
        AyaHashMap::try_from(
            self.ebpf
                .take_map(map_name)
                .context(missing_map_context(map_name))?,
        )
        .context(map_init_context(map_name))
    }

    fn optional_hash_u32_u8(
        &mut self,
        map_name: &'static str,
    ) -> anyhow::Result<Option<AyaHashMap<MapData, u32, u8>>> {
        self.ebpf
            .take_map(map_name)
            .map(AyaHashMap::try_from)
            .transpose()
            .context(map_init_context(map_name))
    }
}

impl MapInitOps for AyaMapInitOps<'_> {
    type TargetPidMap = AyaHashMap<MapData, u32, u8>;
    type TargetIrqMap = AyaHashMap<MapData, u32, u8>;
    type DropCounters = PerCpuArray<MapData, u64>;
    type Events = AsyncFd<RingBuf<MapData>>;
    type PrevFaultsMap = AyaHashMap<MapData, u32, [u64; 2]>;

    fn target_pid_map(&mut self) -> anyhow::Result<Self::TargetPidMap> {
        self.required_hash_u32_u8(TARGET_PIDS_MAP)
    }

    fn target_irq_map(&mut self) -> anyhow::Result<Option<Self::TargetIrqMap>> {
        self.optional_hash_u32_u8(TARGET_IRQS_MAP)
    }

    fn drop_counters(&mut self) -> anyhow::Result<Self::DropCounters> {
        PerCpuArray::try_from(
            self.ebpf
                .take_map(DROP_COUNTERS_MAP)
                .context(missing_map_context(DROP_COUNTERS_MAP))?,
        )
        .context(map_init_context(DROP_COUNTERS_MAP))
    }

    fn events(&mut self) -> anyhow::Result<Self::Events> {
        let events = RingBuf::try_from(
            self.ebpf
                .take_map(EVENTS_MAP)
                .context(missing_map_context(EVENTS_MAP))?,
        )
        .context(map_init_context(EVENTS_MAP))?;

        AsyncFd::new(events).context("eBPF load failed: events ringbuf async fd")
    }

    fn prev_faults_map(&mut self) -> anyhow::Result<Option<Self::PrevFaultsMap>> {
        self.ebpf
            .take_map(PREV_FAULTS_MAP)
            .map(AyaHashMap::try_from)
            .transpose()
            .context(map_init_context(PREV_FAULTS_MAP))
    }
}
