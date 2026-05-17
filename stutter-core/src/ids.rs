use serde::{Deserialize, Serialize};

macro_rules! numeric_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(
            Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(u32);

        impl $name {
            pub const fn new(value: u32) -> Self {
                Self(value)
            }

            pub const fn as_u32(self) -> u32 {
                self.0
            }
        }

        impl From<u32> for $name {
            fn from(value: u32) -> Self {
                Self::new(value)
            }
        }

        impl From<$name> for u32 {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl TryFrom<u64> for $name {
            type Error = std::num::TryFromIntError;

            fn try_from(value: u64) -> Result<Self, Self::Error> {
                Ok(Self::new(u32::try_from(value)?))
            }
        }

        impl TryFrom<usize> for $name {
            type Error = std::num::TryFromIntError;

            fn try_from(value: usize) -> Result<Self, Self::Error> {
                Ok(Self::new(u32::try_from(value)?))
            }
        }
    };
}

macro_rules! string_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self::new(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self::new(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.into_string()
            }
        }
    };
}

numeric_id!(Pid, "Operating system process identifier.");
numeric_id!(Tid, "Operating system task/thread identifier.");
numeric_id!(CpuId, "Logical CPU identifier.");
numeric_id!(IrqId, "Interrupt request identifier.");
numeric_id!(
    ProcessId,
    "Domain process identifier distinct from raw PID usage."
);
numeric_id!(
    TaskId,
    "Domain task identifier distinct from raw TID usage."
);

string_id!(
    StableId,
    "Stable string identifier for domain objects shared across crate boundaries."
);
string_id!(RunId, "Run/session identifier.");
string_id!(ActionId, "Action identifier.");
string_id!(ExperimentId, "Experiment identifier.");

impl From<Pid> for ProcessId {
    fn from(value: Pid) -> Self {
        Self::new(value.as_u32())
    }
}

impl From<ProcessId> for Pid {
    fn from(value: ProcessId) -> Self {
        Self::new(value.as_u32())
    }
}

impl From<Tid> for TaskId {
    fn from(value: Tid) -> Self {
        Self::new(value.as_u32())
    }
}

impl From<TaskId> for Tid {
    fn from(value: TaskId) -> Self {
        Self::new(value.as_u32())
    }
}

#[cfg(test)]
mod tests {
    use super::{ActionId, CpuId, ExperimentId, IrqId, Pid, ProcessId, RunId, TaskId, Tid};

    #[test]
    fn numeric_ids_construct_and_convert_to_raw_values() {
        let pid = Pid::new(1_234);
        let tid = Tid::from(1_235);
        let cpu = CpuId::new(7);
        let irq = IrqId::from(42);

        assert_eq!(pid.as_u32(), 1_234);
        assert_eq!(u32::from(pid), 1_234);
        assert_eq!(tid.as_u32(), 1_235);
        assert_eq!(u32::from(tid), 1_235);
        assert_eq!(cpu.as_u32(), 7);
        assert_eq!(u32::from(cpu), 7);
        assert_eq!(irq.as_u32(), 42);
        assert_eq!(u32::from(irq), 42);
    }

    #[test]
    fn numeric_ids_try_from_wider_values() {
        let pid = match Pid::try_from(1_234_u64) {
            Ok(pid) => pid,
            Err(err) => panic!("expected valid pid conversion, got {err}"),
        };
        assert_eq!(pid.as_u32(), 1_234);

        let cpu = match CpuId::try_from(7_usize) {
            Ok(cpu) => cpu,
            Err(err) => panic!("expected valid cpu conversion, got {err}"),
        };
        assert_eq!(cpu.as_u32(), 7);

        assert!(Pid::try_from(u64::from(u32::MAX) + 1).is_err());
    }

    #[test]
    fn process_and_task_ids_convert_explicitly_from_pid_and_tid() {
        let process = ProcessId::from(Pid::new(2_000));
        let pid = Pid::from(process);
        assert_eq!(pid.as_u32(), 2_000);

        let task = TaskId::from(Tid::new(2_001));
        let tid = Tid::from(task);
        assert_eq!(tid.as_u32(), 2_001);
    }

    #[test]
    fn string_ids_construct_and_convert_to_owned_strings() {
        let run = RunId::from("run-001");
        assert_eq!(run.as_str(), "run-001");
        assert_eq!(String::from(run), "run-001");

        let action = ActionId::from(String::from("action/cpu-affinity"));
        assert_eq!(action.as_str(), "action/cpu-affinity");
        assert_eq!(action.into_string(), "action/cpu-affinity");

        let experiment = ExperimentId::new("experiment/live-001");
        assert_eq!(experiment.as_str(), "experiment/live-001");
    }
}
