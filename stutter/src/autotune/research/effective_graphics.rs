//! What the verified results show of the graphics configuration the frozen profile asked for, surfaced in the bundle so
//! an absence is a typed `Unproven` rather than a silent pass: per child, how many completed attempts confirmed the
//! profile's rendered resolution (with dynamic scaling off), contradicted it, did not report it, or were of a profile
//! that asks for none. A contradiction is already a profile violation; this makes the unproven ones visible.

use std::{collections::BTreeMap, fs::File, os::fd::AsFd};

use serde_json::{Value, json};

use super::{
    child_ledger::Disposition, native_context::EffectiveResolution, report_store::read_evidence,
};

/// The counts for one child, or `None` when its evidence cannot be read or it is not in the frozen plan.
pub fn of_child(
    studies: &File,
    name: &str,
    uid: u32,
    dispositions: &[Disposition],
) -> Option<Value> {
    let disposition = dispositions
        .iter()
        .find(|study| study.study_sha256.starts_with(name))?;
    let dir = File::from(crate::syscall::openat_dir_nofollow(studies.as_fd(), name).ok()?);
    let evidence = File::from(crate::syscall::openat_dir_nofollow(dir.as_fd(), "evidence").ok()?);
    let items = read_evidence(&evidence, uid).ok()?;
    let mut counts: BTreeMap<&'static str, u32> = BTreeMap::new();
    for item in items.values() {
        let state = match item
            .native_completion
            .as_deref()
            .and_then(|native| native.context.as_ref())
        {
            Some(context) => match context.effective_resolution(disposition.profile) {
                EffectiveResolution::Confirmed => "confirmed",
                EffectiveResolution::Contradicted => "contradicted",
                EffectiveResolution::Unproven => "unproven",
                EffectiveResolution::NotSpecified => "not_specified",
            },
            // A result with no context (a long session) reports nothing of its configuration.
            None => "not_specified",
        };
        *counts.entry(state).or_default() += 1;
    }
    Some(json!({
        "profile": disposition.profile,
        "confirmed": counts.get("confirmed").copied().unwrap_or(0),
        "contradicted": counts.get("contradicted").copied().unwrap_or(0),
        "unproven": counts.get("unproven").copied().unwrap_or(0),
        "not_specified": counts.get("not_specified").copied().unwrap_or(0),
    }))
}

/// The campaign's totals over every verified child, and whether any attempt of a profile that asks for a resolution
/// left it unproven (the bundle then says so instead of implying it was checked).
pub fn summarize(response: &Value) -> Value {
    let mut total = [0u64; 4];
    if let Some(children) = response
        .pointer("/child_reports/children")
        .and_then(Value::as_array)
    {
        for child in children {
            let Some(graphics) = child.get("effective_graphics") else {
                continue;
            };
            for (slot, key) in ["confirmed", "contradicted", "unproven", "not_specified"]
                .into_iter()
                .enumerate()
            {
                total[slot] = total[slot].saturating_add(graphics[key].as_u64().unwrap_or(0));
            }
        }
    }
    json!({
        "confirmed": total[0], "contradicted": total[1], "unproven": total[2], "not_specified": total[3],
        "all_resolutions_confirmed": total[0] > 0 && total[1] == 0 && total[2] == 0,
        "note": "unproven means the game's own result did not report the rendered resolution; it is never assumed to match",
    })
}

#[cfg(test)]
#[path = "effective_graphics_tests.rs"]
mod tests;
