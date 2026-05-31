use std::{borrow::Borrow, fmt};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
#[error("{type_name} cannot be empty")]
pub struct EmptyStringIdError {
    type_name: &'static str,
}

impl EmptyStringIdError {
    pub const fn new(type_name: &'static str) -> Self {
        Self { type_name }
    }

    pub const fn type_name(self) -> &'static str {
        self.type_name
    }
}

macro_rules! numeric_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(
            Clone,
            Copy,
            Debug,
            Default,
            Eq,
            PartialEq,
            Ord,
            PartialOrd,
            Hash,
            Serialize,
            Deserialize,
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

        impl From<$name> for u64 {
            fn from(value: $name) -> Self {
                u64::from(value.0)
            }
        }

        impl From<$name> for usize {
            fn from(value: $name) -> Self {
                value.0 as usize
            }
        }

        impl PartialEq<u32> for $name {
            fn eq(&self, other: &u32) -> bool {
                self.0 == *other
            }
        }

        impl PartialEq<$name> for u32 {
            fn eq(&self, other: &$name) -> bool {
                *self == other.0
            }
        }

        impl Borrow<u32> for $name {
            fn borrow(&self) -> &u32 {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
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
            /// Construct a string identifier without validation.
            ///
            /// This constructor is retained for compatibility with existing internal
            /// call sites. New user/input-facing code should prefer [`Self::try_new`].
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            /// Construct a string identifier, rejecting empty or whitespace-only values.
            pub fn try_new(value: impl Into<String>) -> Result<Self, EmptyStringIdError> {
                let value = value.into();
                if value.trim().is_empty() {
                    return Err(EmptyStringIdError::new(stringify!($name)));
                }
                Ok(Self(value))
            }

            /// Validate an already-constructed or deserialized identifier.
            ///
            /// This is useful after serde deserialization, because these IDs use
            /// `#[serde(transparent)]` to preserve the existing JSON string shape.
            pub fn validate_non_empty(&self) -> Result<(), EmptyStringIdError> {
                if self.0.trim().is_empty() {
                    return Err(EmptyStringIdError::new(stringify!($name)));
                }
                Ok(())
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

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
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
    use super::{
        ActionId, CpuId, EmptyStringIdError, ExperimentId, IrqId, Pid, ProcessId, RunId, StableId,
        TaskId, Tid,
    };

    #[test]
    fn string_ids_try_new_rejects_empty_and_whitespace_values() {
        for err in [
            StableId::try_new("").expect_err("empty StableId should fail"),
            RunId::try_new("").expect_err("empty RunId should fail"),
            ActionId::try_new("   ").expect_err("whitespace ActionId should fail"),
            ExperimentId::try_new("\t\n").expect_err("whitespace ExperimentId should fail"),
        ] {
            assert!(matches!(err, EmptyStringIdError { .. }));
        }

        assert_eq!(
            RunId::try_new("")
                .expect_err("empty RunId should fail")
                .type_name(),
            "RunId"
        );
        assert_eq!(
            ActionId::try_new(" ")
                .expect_err("empty ActionId should fail")
                .type_name(),
            "ActionId"
        );
        assert_eq!(
            ExperimentId::try_new("\n")
                .expect_err("empty ExperimentId should fail")
                .type_name(),
            "ExperimentId"
        );
    }

    #[test]
    fn string_ids_try_new_accepts_non_empty_values_without_trimming() {
        let run = RunId::try_new(" run-001 ").expect("non-empty RunId should pass");
        assert_eq!(run.as_str(), " run-001 ");

        let action =
            ActionId::try_new("action/cpu-affinity").expect("non-empty ActionId should pass");
        assert_eq!(action.as_str(), "action/cpu-affinity");

        let experiment = ExperimentId::try_new("experiment/live-001")
            .expect("non-empty ExperimentId should pass");
        assert_eq!(experiment.as_str(), "experiment/live-001");
    }

    #[test]
    fn compatibility_string_id_new_still_accepts_empty_until_call_sites_migrate() {
        let run = RunId::new("");
        assert_eq!(run.as_str(), "");
        assert!(run.validate_non_empty().is_err());
    }

    #[test]
    fn numeric_ids_construct_and_convert_to_raw_values() {
        let pid = Pid::new(1_234);
        let tid = Tid::from(1_235);
        let cpu = CpuId::new(7);
        let irq = IrqId::from(42);

        assert_eq!(pid.as_u32(), 1_234);
        assert_eq!(u32::from(pid), 1_234);
        assert_eq!(u64::from(pid), 1_234);
        assert_eq!(pid, 1_234);
        assert_eq!(format!("{pid}"), "1234");
        assert_eq!(tid.as_u32(), 1_235);
        assert_eq!(u32::from(tid), 1_235);
        assert_eq!(usize::from(tid), 1_235);
        assert_eq!(tid, 1_235);
        assert_eq!(cpu.as_u32(), 7);
        assert_eq!(u32::from(cpu), 7);
        assert_eq!(cpu, 7);
        assert_eq!(irq.as_u32(), 42);
        assert_eq!(u32::from(irq), 42);
        assert_eq!(irq, 42);
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

    #[test]
    fn string_ids_display_as_inner_strings() {
        let action = ActionId::try_new("cpu-affinity-profile:game").unwrap();
        let experiment = ExperimentId::try_new("experiment-1").unwrap();

        assert_eq!(action.to_string(), "cpu-affinity-profile:game");
        assert_eq!(experiment.to_string(), "experiment-1");
    }
}
