//! Cold arms are initialized first: when the intent froze any, the plan opens with one `Init` study and
//! every screen depends on it, so a screen is never planned (or digested) apart from the initialization
//! it rests on. An intent with no cold arms plans exactly what it always did.

use super::{ChildStudy, Profile, ResearchIntentV1, Stage, study};

pub(super) fn study_before_screens(
    intent: &ResearchIntentV1,
    digest: &str,
    studies: &mut Vec<ChildStudy>,
) -> anyhow::Result<Vec<String>> {
    if intent.prefix_init_arms.is_empty() {
        return Ok(Vec::new());
    }
    let init = study(
        digest,
        Profile::Cpu,
        Stage::Init,
        u32::try_from(intent.prefix_init_arms.len())?,
        Vec::new(),
    )?;
    let digests = vec![init.study_sha256.clone()];
    studies.push(init);
    Ok(digests)
}
