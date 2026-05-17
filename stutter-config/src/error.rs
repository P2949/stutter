use std::{error::Error, fmt};

/// Error type for shared configuration resolution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigError {
    InvalidLayer { message: String },
}

impl ConfigError {
    pub fn invalid_layer(message: impl Into<String>) -> Self {
        Self::InvalidLayer {
            message: message.into(),
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLayer { message } => {
                write!(formatter, "invalid config layer: {message}")
            }
        }
    }
}

impl Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::ConfigError;

    #[test]
    fn invalid_layer_error_formats_message() {
        let error = ConfigError::invalid_layer("missing required path");
        assert_eq!(
            error.to_string(),
            "invalid config layer: missing required path"
        );
    }
}
