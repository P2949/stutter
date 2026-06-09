use super::{
    model::{IoPrioClass, IoPrioValue},
    preflight::validate_ioprio_value,
};
use crate::actions::ActionBoundaryError;

pub(crate) const IOPRIO_CLASS_SHIFT: i32 = 13;
pub(crate) const IOPRIO_PRIO_MASK: i32 = (1 << IOPRIO_CLASS_SHIFT) - 1;

impl IoPrioClass {
    pub(crate) fn linux_class(self) -> i32 {
        match self {
            Self::None => 0,
            Self::Realtime => 1,
            Self::BestEffort => 2,
            Self::Idle => 3,
        }
    }

    pub(crate) fn from_linux_class(class: i32) -> anyhow::Result<Self> {
        match class {
            0 => Ok(Self::None),
            1 => Ok(Self::Realtime),
            2 => Ok(Self::BestEffort),
            3 => Ok(Self::Idle),
            other => Err(ActionBoundaryError::UnsupportedValue {
                action_kind: "ioprio",
                field: "linux_class",
                value: other.to_string(),
            }
            .into()),
        }
    }
}

impl IoPrioValue {
    pub fn encode(self) -> anyhow::Result<i32> {
        validate_ioprio_value(self)?;
        Ok((self.class.linux_class() << IOPRIO_CLASS_SHIFT) | i32::from(self.level.unwrap_or(0)))
    }

    pub fn decode(encoded: i32) -> anyhow::Result<Self> {
        if encoded < 0 {
            return Err(ActionBoundaryError::InvalidValue {
                action_kind: "ioprio",
                field: "encoded_ioprio".to_owned(),
                reason: format!("negative encoded I/O priority {encoded}"),
            }
            .into());
        }

        let class = IoPrioClass::from_linux_class(encoded >> IOPRIO_CLASS_SHIFT)?;
        let data = (encoded & IOPRIO_PRIO_MASK) as u8;

        let level = match class {
            IoPrioClass::BestEffort | IoPrioClass::Realtime => Some(data),
            IoPrioClass::None | IoPrioClass::Idle => None,
        };

        let value = Self { class, level };
        validate_ioprio_value(value)?;
        Ok(value)
    }
}
