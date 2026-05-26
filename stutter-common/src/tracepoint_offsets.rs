#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TracepointName<'a>(&'a str);

impl<'a> TracepointName<'a> {
    pub const fn new(value: &'a str) -> Self {
        Self(value)
    }

    pub const fn as_str(self) -> &'a str {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TracepointFieldName<'a>(&'a str);

impl<'a> TracepointFieldName<'a> {
    pub const fn new(value: &'a str) -> Self {
        Self(value)
    }

    pub const fn as_str(self) -> &'a str {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TracepointFieldSpec {
    pub name: TracepointFieldName<'static>,
    pub offset: usize,
}

impl TracepointFieldSpec {
    pub const fn new(name: &'static str, offset: usize) -> Self {
        Self {
            name: TracepointFieldName::new(name),
            offset,
        }
    }
}

#[cfg(feature = "user")]
pub const TRACEPOINT_SCHED_WAKEUP: TracepointName<'static> = TracepointName::new("sched_wakeup");
#[cfg(feature = "user")]
pub const TRACEPOINT_SCHED_SWITCH: TracepointName<'static> = TracepointName::new("sched_switch");
#[cfg(feature = "user")]
pub const TRACEPOINT_SCHED_WAKEUP_NEW: TracepointName<'static> =
    TracepointName::new("sched_wakeup_new");
#[cfg(feature = "user")]
pub const TRACEPOINT_SCHED_MIGRATE_TASK: TracepointName<'static> =
    TracepointName::new("sched_migrate_task");
#[cfg(feature = "user")]
pub const TRACEPOINT_CPU_FREQUENCY: TracepointName<'static> = TracepointName::new("cpu_frequency");
#[cfg(feature = "user")]
pub const TRACEPOINT_SCHED_STAT_WAIT: TracepointName<'static> =
    TracepointName::new("sched_stat_wait");
#[cfg(feature = "user")]
pub const TRACEPOINT_IRQ_HANDLER_ENTRY: TracepointName<'static> =
    TracepointName::new("irq_handler_entry");
#[cfg(feature = "user")]
pub const TRACEPOINT_IRQ_HANDLER_EXIT: TracepointName<'static> =
    TracepointName::new("irq_handler_exit");
#[cfg(feature = "user")]
pub const TRACEPOINT_SCHED_PROCESS_EXIT: TracepointName<'static> =
    TracepointName::new("sched_process_exit");
#[cfg(feature = "user")]
pub const TRACEPOINT_SCHED_PROCESS_EXEC: TracepointName<'static> =
    TracepointName::new("sched_process_exec");

pub const SCHED_WAKEUP_PID_OFFSET: usize = 24;
pub const SCHED_WAKEUP_PRIO_OFFSET: usize = 28;
pub const SCHED_WAKEUP_TARGET_CPU_OFFSET: usize = 32;

pub const SCHED_SWITCH_PREV_PID_OFFSET: usize = 24;
pub const SCHED_SWITCH_PREV_STATE_OFFSET: usize = 32;
pub const SCHED_SWITCH_NEXT_COMM_OFFSET: usize = 40;
pub const SCHED_SWITCH_NEXT_PID_OFFSET: usize = 56;
pub const SCHED_SWITCH_NEXT_PRIO_OFFSET: usize = 60;

pub const SCHED_MIGRATE_TASK_PID_OFFSET: usize = 12;
pub const SCHED_MIGRATE_TASK_ORIG_CPU_OFFSET: usize = 20;
pub const SCHED_MIGRATE_TASK_DEST_CPU_OFFSET: usize = 24;

pub const CPU_FREQUENCY_STATE_OFFSET: usize = 8;
pub const CPU_FREQUENCY_CPU_ID_OFFSET: usize = 12;

pub const SCHED_STAT_WAIT_PID_OFFSET: usize = 8;
pub const SCHED_STAT_WAIT_DELAY_OFFSET: usize = 16;

pub const IRQ_HANDLER_IRQ_OFFSET: usize = 8;

#[cfg(feature = "user")]
pub const SCHED_WAKEUP_FIELDS: &[TracepointFieldSpec] = &[
    TracepointFieldSpec::new("pid", SCHED_WAKEUP_PID_OFFSET),
    TracepointFieldSpec::new("prio", SCHED_WAKEUP_PRIO_OFFSET),
    TracepointFieldSpec::new("target_cpu", SCHED_WAKEUP_TARGET_CPU_OFFSET),
];

#[cfg(feature = "user")]
pub const SCHED_SWITCH_FIELDS: &[TracepointFieldSpec] = &[
    TracepointFieldSpec::new("prev_pid", SCHED_SWITCH_PREV_PID_OFFSET),
    TracepointFieldSpec::new("prev_state", SCHED_SWITCH_PREV_STATE_OFFSET),
    TracepointFieldSpec::new("next_comm", SCHED_SWITCH_NEXT_COMM_OFFSET),
    TracepointFieldSpec::new("next_pid", SCHED_SWITCH_NEXT_PID_OFFSET),
    TracepointFieldSpec::new("next_prio", SCHED_SWITCH_NEXT_PRIO_OFFSET),
];

#[cfg(feature = "user")]
pub const SCHED_MIGRATE_TASK_FIELDS: &[TracepointFieldSpec] = &[
    TracepointFieldSpec::new("pid", SCHED_MIGRATE_TASK_PID_OFFSET),
    TracepointFieldSpec::new("orig_cpu", SCHED_MIGRATE_TASK_ORIG_CPU_OFFSET),
    TracepointFieldSpec::new("dest_cpu", SCHED_MIGRATE_TASK_DEST_CPU_OFFSET),
];

#[cfg(feature = "user")]
pub const CPU_FREQUENCY_FIELDS: &[TracepointFieldSpec] = &[
    TracepointFieldSpec::new("state", CPU_FREQUENCY_STATE_OFFSET),
    TracepointFieldSpec::new("cpu_id", CPU_FREQUENCY_CPU_ID_OFFSET),
];

#[cfg(feature = "user")]
pub const SCHED_STAT_WAIT_FIELDS: &[TracepointFieldSpec] = &[
    TracepointFieldSpec::new("pid", SCHED_STAT_WAIT_PID_OFFSET),
    TracepointFieldSpec::new("delay", SCHED_STAT_WAIT_DELAY_OFFSET),
];

#[cfg(feature = "user")]
pub const IRQ_HANDLER_FIELDS: &[TracepointFieldSpec] =
    &[TracepointFieldSpec::new("irq", IRQ_HANDLER_IRQ_OFFSET)];

pub const BLOCK_RQ_DEV_OFFSET: usize = 8;
pub const BLOCK_RQ_SECTOR_OFFSET: usize = 16;

pub const BLOCK_RQ_DEV_MIN_SIZE: u32 = 4;
pub const BLOCK_RQ_SECTOR_MIN_SIZE: u32 = 8;
pub const BLOCK_RQ_REQUEST_POINTER_MIN_SIZE: u32 = 8;
pub const BLOCK_RQ_NR_SECTOR_MIN_SIZE: u32 = 4;
pub const BLOCK_RQ_RWBS_MIN_SIZE: u32 = 8;

#[cfg(feature = "user")]
pub const BLOCK_RQ_REQUIRED_METADATA_FIELDS: &[(&str, usize, u32)] = &[
    ("dev", BLOCK_RQ_DEV_OFFSET, BLOCK_RQ_DEV_MIN_SIZE),
    ("sector", BLOCK_RQ_SECTOR_OFFSET, BLOCK_RQ_SECTOR_MIN_SIZE),
];
