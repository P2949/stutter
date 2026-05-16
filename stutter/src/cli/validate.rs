use std::path::PathBuf;

use clap::Args;
use serde::Serialize;

#[derive(Args, Clone, Debug, Serialize)]
pub struct ValidateArgs {
    pub file: PathBuf,
}
