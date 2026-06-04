use super::*;

#[derive(Args, Debug, Clone)]
pub(super) struct DoctorArgs {
    #[arg(long)]
    pub(super) json: bool,

    #[command(subcommand)]
    pub(super) command: Option<DoctorSubcommand>,

    #[arg(long = "hwmon", id = "hwmon")]
    pub(super) hwmon: bool,

    #[arg(long = "hwmon-root", value_name = "PATH", requires = "hwmon")]
    pub(super) hwmon_root: Option<PathBuf>,

    #[arg(long = "hwmon-drm-card", value_name = "CARD", requires = "hwmon")]
    pub(super) hwmon_drm_card: Option<String>,

    #[arg(long = "hwmon-render-node", value_name = "NODE", requires = "hwmon")]
    pub(super) hwmon_render_node: Option<PathBuf>,

    #[arg(long = "irq-latency")]
    pub(super) irq_latency: bool,

    #[arg(long = "irq", value_name = "IRQ")]
    pub(super) irqs: Vec<u32>,

    #[arg(long = "block-io")]
    pub(super) block_io: bool,

    #[arg(long = "kms-timing")]
    pub(super) kms_timing: bool,

    #[arg(long = "faults")]
    pub(super) faults: bool,

    #[arg(long = "cpu-perf")]
    pub(super) cpu_perf: bool,

    #[arg(long = "mangohud-log", value_name = "PATH")]
    pub(super) mangohud_log: Option<PathBuf>,
}

#[derive(Subcommand, Debug, Clone)]
pub(super) enum DoctorSubcommand {
    Tracepoints(DoctorTracepointsArgs),
}

#[derive(Args, Debug, Clone)]
pub(super) struct DoctorTracepointsArgs {
    #[arg(long)]
    pub(super) dump: bool,

    #[arg(
        long = "events-root",
        value_name = "PATH",
        help = "Tracefs events root to dump; defaults to /sys/kernel/tracing/events"
    )]
    pub(super) events_root: Option<PathBuf>,
}

impl DoctorArgs {
    pub(super) fn into_input(self) -> anyhow::Result<crate::doctor::DoctorInput> {
        let mut tracepoint_events_root = None;

        let tracepoint_dump = match self.command {
            Some(DoctorSubcommand::Tracepoints(tracepoints)) => {
                if !tracepoints.dump {
                    anyhow::bail!("doctor tracepoints requires --dump");
                }
                tracepoint_events_root = tracepoints.events_root;
                true
            }
            None => false,
        };

        Ok(crate::doctor::DoctorInput {
            json: self.json,
            tracepoint_dump,
            tracepoint_events_root,
            hwmon: self.hwmon,
            hwmon_root: self.hwmon_root,
            hwmon_drm_card: self.hwmon_drm_card,
            hwmon_render_node: self.hwmon_render_node,
            irq_latency: self.irq_latency,
            irqs: self.irqs,
            block_io: self.block_io,
            kms_timing: self.kms_timing,
            faults: self.faults,
            cpu_perf: self.cpu_perf,
            mangohud_log: self.mangohud_log,
        })
    }
}
