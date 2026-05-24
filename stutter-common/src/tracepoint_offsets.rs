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
pub const SCHED_WAKEUP_FIELDS: &[(&str, usize)] = &[
    ("pid", SCHED_WAKEUP_PID_OFFSET),
    ("prio", SCHED_WAKEUP_PRIO_OFFSET),
    ("target_cpu", SCHED_WAKEUP_TARGET_CPU_OFFSET),
];

#[cfg(feature = "user")]
pub const SCHED_SWITCH_FIELDS: &[(&str, usize)] = &[
    ("prev_pid", SCHED_SWITCH_PREV_PID_OFFSET),
    ("prev_state", SCHED_SWITCH_PREV_STATE_OFFSET),
    ("next_comm", SCHED_SWITCH_NEXT_COMM_OFFSET),
    ("next_pid", SCHED_SWITCH_NEXT_PID_OFFSET),
    ("next_prio", SCHED_SWITCH_NEXT_PRIO_OFFSET),
];

#[cfg(feature = "user")]
pub const SCHED_MIGRATE_TASK_FIELDS: &[(&str, usize)] = &[
    ("pid", SCHED_MIGRATE_TASK_PID_OFFSET),
    ("orig_cpu", SCHED_MIGRATE_TASK_ORIG_CPU_OFFSET),
    ("dest_cpu", SCHED_MIGRATE_TASK_DEST_CPU_OFFSET),
];

#[cfg(feature = "user")]
pub const CPU_FREQUENCY_FIELDS: &[(&str, usize)] = &[
    ("state", CPU_FREQUENCY_STATE_OFFSET),
    ("cpu_id", CPU_FREQUENCY_CPU_ID_OFFSET),
];

#[cfg(feature = "user")]
pub const SCHED_STAT_WAIT_FIELDS: &[(&str, usize)] = &[
    ("pid", SCHED_STAT_WAIT_PID_OFFSET),
    ("delay", SCHED_STAT_WAIT_DELAY_OFFSET),
];

#[cfg(feature = "user")]
pub const IRQ_HANDLER_FIELDS: &[(&str, usize)] = &[("irq", IRQ_HANDLER_IRQ_OFFSET)];
