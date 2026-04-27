use aya::{
    Ebpf,
    maps::{HashMap as AyaHashMap, MapData, RingBuf},
    programs::TracePoint,
};
use tokio::io::unix::AsyncFd;

pub struct LoadedEbpf {
    #[allow(dead_code)]
    ebpf: Ebpf,
    pub events: AsyncFd<RingBuf<MapData>>,
    pub target_pid_map: AyaHashMap<MapData, u32, u8>,
}

pub fn load_and_attach() -> anyhow::Result<LoadedEbpf> {
    raise_memlock_limit();

    let mut ebpf = Ebpf::load(aya::include_bytes_aligned!(concat!(
        env!("OUT_DIR"),
        "/stutter"
    )))?;

    attach_tracepoint(&mut ebpf, "sched_wakeup", "sched", "sched_wakeup")?;
    attach_tracepoint(&mut ebpf, "sched_switch", "sched", "sched_switch")?;
    attach_tracepoint(
        &mut ebpf,
        "sched_process_exit",
        "sched",
        "sched_process_exit",
    )?;

    let target_pid_map = AyaHashMap::try_from(
        ebpf.take_map("TARGET_PIDS")
            .ok_or_else(|| anyhow::anyhow!("TARGET_PIDS map not found"))?,
    )?;

    let events = RingBuf::try_from(
        ebpf.take_map("EVENTS")
            .ok_or_else(|| anyhow::anyhow!("EVENTS map not found"))?,
    )?;

    let events = AsyncFd::new(events)?;

    Ok(LoadedEbpf {
        ebpf,
        events,
        target_pid_map,
    })
}

fn attach_tracepoint(
    ebpf: &mut Ebpf,
    program_name: &str,
    category: &str,
    tracepoint_name: &str,
) -> anyhow::Result<()> {
    let program: &mut TracePoint = ebpf
        .program_mut(program_name)
        .ok_or_else(|| anyhow::anyhow!("{program_name} program not found"))?
        .try_into()?;

    program.load()?;
    program.attach(category, tracepoint_name)?;

    Ok(())
}

fn raise_memlock_limit() {
    let rlim = libc::rlimit {
        rlim_cur: libc::RLIM_INFINITY,
        rlim_max: libc::RLIM_INFINITY,
    };

    let ret = unsafe { libc::setrlimit(libc::RLIMIT_MEMLOCK, &rlim) };
    if ret != 0 {
        eprintln!("warning: failed to raise RLIMIT_MEMLOCK");
    }
}
