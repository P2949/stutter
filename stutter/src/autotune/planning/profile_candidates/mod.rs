//! CPU-affinity profile candidate generation.

pub use rules::*;
#[cfg(test)]
pub(crate) use validate::validate_generated_profile;

use super::candidate::CandidateAction;
use crate::profiles::Profile;

#[derive(Clone, Debug)]
pub struct GeneratedProfileCandidatePlan {
    pub optimization_candidates: Vec<CandidateAction>,
    pub recovery_fallback: Option<CandidateAction>,
    pub rejected: Vec<RejectedCandidateProfile>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeneratedCpuSetPolicy {
    pub allowed_cpus: Option<crate::affinity::CpuMask>,
    pub denied_cpus: Option<crate::affinity::CpuMask>,
    pub min_render_cpus: usize,
    pub min_game_cpus: usize,
    pub min_compositor_cpus: usize,
    pub min_background_cpus: usize,
}

impl Default for GeneratedCpuSetPolicy {
    fn default() -> Self {
        Self {
            allowed_cpus: None,
            denied_cpus: None,
            min_render_cpus: 1,
            min_game_cpus: 1,
            min_compositor_cpus: 1,
            min_background_cpus: 2,
        }
    }
}

#[derive(Clone, Debug)]
pub struct GeneratedTopologyProfilePlan {
    pub profiles: Vec<Profile>,
    pub rejected: Vec<RejectedCandidateProfile>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RejectedCandidateProfile {
    pub profile_name: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateProfileStatus {
    pub matched_tasks: usize,
    pub dry_run_tasks: usize,
}

pub mod gaming;
pub mod helpers;
pub mod rules;
pub mod topology;
pub mod validate;
