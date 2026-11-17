//! Cold or warm identity of each arm's Wine prefix (PLAN section 6.1). A separate directory per
//! Proton arm stops arms sharing state, but a prefix that already ran the game (warm: shader and
//! registry state, previous Proton version stamps) is not comparable with a freshly created one
//! (cold), and an arm that was once run under a different Proton build carries that build's
//! stamps. The observation is read-only and cheap; a comparison over arms in different classes, or
//! over a warm prefix last stamped by a different Proton than the arm's own, is refused.

use std::{
    io::Read,
    path::{Path, PathBuf},
};

/// The files that exist once a prefix has been initialised by a Proton run.
const REGISTRY_FILES: [&str; 3] = ["system.reg", "user.reg", "userdef.reg"];
/// Proton writes the build that last used a prefix here (relative to the arm's compat directory).
const VERSION_STAMP: &str = "version";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrefixClass {
    /// No prefix exists yet: the first run creates it.
    Cold,
    /// A prefix that a previous run initialised.
    Warm,
    /// Partly initialised or unreadable: neither cold nor warm, so not comparable with anything.
    Indeterminate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrefixObservation {
    pub class: PrefixClass,
    /// The Proton build stamp the prefix carries (`None` when cold or unstamped).
    pub stamped_by: Option<String>,
}

fn read_stamp(path: &Path) -> Option<String> {
    let mut text = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(4096)
        .read_to_string(&mut text)
        .ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.chars().take(256).collect())
}

/// Observe one arm's compat directory without creating or changing anything.
pub fn observe(compat_dir: &Path) -> PrefixObservation {
    let pfx: PathBuf = compat_dir.join("pfx");
    let present = REGISTRY_FILES
        .iter()
        .filter(|name| pfx.join(name).is_file())
        .count();
    let class = match (pfx.exists(), present) {
        (false, _) => PrefixClass::Cold,
        (true, 3) => PrefixClass::Warm,
        (true, _) => PrefixClass::Indeterminate,
    };
    PrefixObservation {
        class,
        stamped_by: (class == PrefixClass::Warm)
            .then(|| read_stamp(&compat_dir.join(VERSION_STAMP)))
            .flatten(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Incomparable {
    /// Some arm's prefix is neither cold nor warm.
    Indeterminate(usize),
    /// Arms differ in class.
    MixedColdAndWarm,
    /// A warm prefix is unstamped, so which Proton last used it is unknown.
    UnstampedWarm(usize),
    /// A warm prefix was last stamped by a different Proton than the arm's own.
    ForeignStamp(usize),
}

/// `arms` pairs each arm's observation with the stamp its own Proton build writes.
pub fn comparable(arms: &[(PrefixObservation, &str)]) -> Result<PrefixClass, Incomparable> {
    let Some((first, _)) = arms.first() else {
        return Ok(PrefixClass::Cold);
    };
    for (index, (observation, _)) in arms.iter().enumerate() {
        if observation.class == PrefixClass::Indeterminate {
            return Err(Incomparable::Indeterminate(index));
        }
    }
    if arms.iter().any(|(o, _)| o.class != first.class) {
        return Err(Incomparable::MixedColdAndWarm);
    }
    if first.class == PrefixClass::Warm {
        for (index, (observation, own_stamp)) in arms.iter().enumerate() {
            match observation.stamped_by.as_deref() {
                None => return Err(Incomparable::UnstampedWarm(index)),
                Some(stamp) if stamp != *own_stamp => {
                    return Err(Incomparable::ForeignStamp(index));
                }
                Some(_) => {}
            }
        }
    }
    Ok(first.class)
}

/// The stamp a Proton build writes into a prefix it has used: the `CURRENT_PREFIX_VERSION` its own
/// `proton` script declares (for example `11.0-100` or `CachyOS-11.0-100`). `None` when the script
/// cannot be read or declares none, so a prefix can then not be shown to belong to this build.
pub fn own_stamp(tool_dir: &Path) -> Option<String> {
    let mut script = String::new();
    std::fs::File::open(tool_dir.join("proton"))
        .ok()?
        .take(4 * 1024 * 1024)
        .read_to_string(&mut script)
        .ok()?;
    script.lines().find_map(|line| {
        let value = line.strip_prefix("CURRENT_PREFIX_VERSION")?.trim_start();
        let value = value.strip_prefix('=')?.trim();
        let value = value.trim_matches(|c| c == '"' || c == '\'');
        (!value.is_empty() && value.len() <= 256).then(|| value.to_owned())
    })
}

/// The comparability policy over the arms a study runs, each as (its compat directory, its Proton
/// installation): every prefix cold, or every prefix warm and stamped by its own build. The reason
/// is a stable code; nothing is changed or created.
pub fn check(arms: &[(PathBuf, PathBuf)]) -> Result<PrefixClass, &'static str> {
    let stamps: Vec<String> = arms
        .iter()
        .map(|(_, tool)| own_stamp(tool).ok_or("PrefixOwnStampUnknown"))
        .collect::<Result<_, _>>()?;
    let observed: Vec<(PrefixObservation, &str)> = arms
        .iter()
        .zip(&stamps)
        .map(|((compat, _), stamp)| (observe(compat), stamp.as_str()))
        .collect();
    comparable(&observed).map_err(|refusal| match refusal {
        Incomparable::Indeterminate(_) => "PrefixIndeterminate",
        Incomparable::MixedColdAndWarm => "PrefixMixedColdAndWarm",
        Incomparable::UnstampedWarm(_) => "PrefixUnstamped",
        Incomparable::ForeignStamp(_) => "PrefixForeignStamp",
    })
}

impl PrefixClass {
    /// The label recorded beside a study so its cohort is visible in the bundle.
    pub fn label(self) -> &'static str {
        match self {
            Self::Cold => "cold",
            Self::Warm => "warm",
            Self::Indeterminate => "indeterminate",
        }
    }
}

#[cfg(test)]
#[path = "prefix_identity_tests.rs"]
mod tests;
