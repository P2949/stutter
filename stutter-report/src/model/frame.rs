use serde::{Deserialize, Serialize};

use super::Diagnosis;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FrameDiagnosis {
    pub frame_elapsed_ms: u64,
    pub frametime_ms: f64,
    pub diagnosis: Diagnosis,
}
