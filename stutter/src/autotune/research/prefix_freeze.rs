//! Freezing the cold arms into an intent before it is submitted. A campaign initializes exactly the arms
//! whose Wine prefix is cold at issue time (`ResearchIntentV1.prefix_init_arms`); this reads each
//! authorized tool's compat directory, read-only, and returns the intent with that list filled in. A
//! prefix that is neither cold nor warm (partly created, unreadable) cannot be initialized or compared,
//! so it is refused rather than guessed.

use std::path::Path;

use super::{
    ResearchIntentV1,
    prefix_identity::{PrefixClass, observe},
};

/// The intent with `prefix_init_arms` set to its cold arms (in the intent's own tool order).
pub fn with_cold_arms(
    intent: &ResearchIntentV1,
    compat_root: &Path,
) -> anyhow::Result<ResearchIntentV1> {
    let mut frozen = intent.clone();
    frozen.prefix_init_arms = Vec::new();
    for tool in &intent.tools {
        let compat = compat_root.join(tool.source.dir_name());
        match observe(&compat).class {
            PrefixClass::Cold => frozen.prefix_init_arms.push(tool.source),
            PrefixClass::Warm => {}
            PrefixClass::Indeterminate => anyhow::bail!(
                "the {} prefix is partly initialized: remove it or finish initializing it first",
                tool.source.dir_name()
            ),
        }
    }
    frozen.validate()?;
    Ok(frozen)
}

#[cfg(test)]
#[path = "prefix_freeze_tests.rs"]
mod tests;
