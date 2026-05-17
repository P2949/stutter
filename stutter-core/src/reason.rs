use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Stable reason code for policy decisions, denials, and diagnostics.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReasonCode(String);

impl ReasonCode {
    pub fn new(value: impl Into<String>) -> Result<Self, ReasonCodeError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(ReasonCodeError::Empty);
        }

        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl TryFrom<String> for ReasonCode {
    type Error = ReasonCodeError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ReasonCodeError {
    #[error("reason code must not be empty")]
    Empty,
}
