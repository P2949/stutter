use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Stable generic reason model for policy decisions, denials, and diagnostics.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct Reason {
    pub code: String,
    pub message: String,
}

impl Reason {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Result<Self, ReasonError> {
        let code =
            ReasonCode::new(code).map_err(|ReasonCodeError::Empty| ReasonError::EmptyCode)?;
        let message = message.into();

        if message.trim().is_empty() {
            return Err(ReasonError::EmptyMessage);
        }

        Ok(Self {
            code: code.into_string(),
            message,
        })
    }

    pub fn from_code(code: ReasonCode, message: impl Into<String>) -> Result<Self, ReasonError> {
        let message = message.into();

        if message.trim().is_empty() {
            return Err(ReasonError::EmptyMessage);
        }

        Ok(Self {
            code: code.into_string(),
            message,
        })
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

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

impl TryFrom<&str> for ReasonCode {
    type Error = ReasonCodeError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ReasonError {
    #[error("reason code must not be empty")]
    EmptyCode,
    #[error("reason message must not be empty")]
    EmptyMessage,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ReasonCodeError {
    #[error("reason code must not be empty")]
    Empty,
}

#[cfg(test)]
mod tests {
    use super::{Reason, ReasonCode, ReasonCodeError, ReasonError};

    #[test]
    fn reason_code_rejects_empty_values() {
        let code = match ReasonCode::new("policy-denied") {
            Ok(code) => code,
            Err(err) => panic!("expected valid reason code, got {err}"),
        };

        assert_eq!(code.as_str(), "policy-denied");
        assert_eq!(ReasonCode::new(" "), Err(ReasonCodeError::Empty));
        assert_eq!(ReasonCode::try_from(""), Err(ReasonCodeError::Empty));
    }

    #[test]
    fn reason_constructs_from_code_and_message() {
        let reason = match Reason::new("policy-denied", "policy denied the candidate") {
            Ok(reason) => reason,
            Err(err) => panic!("expected valid reason, got {err}"),
        };

        assert_eq!(reason.code, "policy-denied");
        assert_eq!(reason.message, "policy denied the candidate");
        assert_eq!(reason.code(), "policy-denied");
        assert_eq!(reason.message(), "policy denied the candidate");
    }

    #[test]
    fn reason_constructs_from_reason_code() {
        let code = match ReasonCode::new("health-warning") {
            Ok(code) => code,
            Err(err) => panic!("expected valid reason code, got {err}"),
        };

        let reason = match Reason::from_code(code, "system health is degraded") {
            Ok(reason) => reason,
            Err(err) => panic!("expected valid reason, got {err}"),
        };

        assert_eq!(reason.code(), "health-warning");
        assert_eq!(reason.message(), "system health is degraded");
    }

    #[test]
    fn reason_rejects_empty_code_or_message() {
        assert_eq!(Reason::new("", "message"), Err(ReasonError::EmptyCode));
        assert_eq!(
            Reason::new("policy-denied", " "),
            Err(ReasonError::EmptyMessage)
        );
    }
}
