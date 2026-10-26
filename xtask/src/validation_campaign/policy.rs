//! The two repository-changing policy tasks: `capability-policy` applies the fixed `v0.1.0` tier
//! policy from the committed manifest to the registry and its documentation, and `release-docs`
//! records the calibration and score decisions. Both land as pull requests through the gated PR
//! flow; the registry edit is a pure text transformation that is verified before it is written.
use std::{fs, path::Path};

use serde_json::Value;

use super::{
    executor::{RunContext, RunnerOutcome, TaskRunner},
    graph::{TaskSpec, validation_gates},
    integration::LEDGER,
    landing_task::LandingTools,
    model::{BlockReason, FailureClass},
};

pub const REGISTRY: &str = "stutter/src/capability_tiers.rs";
pub const CAPABILITIES_DOC: &str = "docs/CAPABILITIES.md";
const START: &str = "<!-- campaign-policy:start -->";
const END: &str = "<!-- campaign-policy:end -->";

/// Issue gates that must be ready before any tier changes (the old human review gate).
pub const REQUIRED_GATES: [&str; 6] = [
    "observation",
    "kernel-compatibility",
    "diagnosis-corpus",
    "diagnosis-calibration",
    "score-evaluation",
    "profiler-overhead",
];

/// Byte range of the definition of capability `id` in the registry source.
fn definition(source: &str, id: &str) -> Option<(usize, usize, bool)> {
    let quoted = format!("\"{id}\"");
    if let Some(at) = source.find(&format!("id: {quoted}")) {
        let start = source[..at].rfind("Capability {")?;
        let end = at + source[at..].find("\n    },")?;
        return Some((start, end, false));
    }
    let mut from = 0;
    while let Some(found) = source[from..].find("obs(") {
        let open = from + found;
        let after = source[open + 4..].trim_start();
        if after.starts_with(&quoted) {
            let mut depth = 0_usize;
            for (offset, ch) in source[open..].char_indices() {
                match ch {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some((open, open + offset, true));
                        }
                    }
                    _ => {}
                }
            }
        }
        from = open + 4;
    }
    None
}

fn set_tiers(
    span: &str,
    obs_call: bool,
    tier: Option<&str>,
    target: &str,
) -> Result<String, String> {
    if obs_call {
        // The call ends with exactly two tier arguments: tier, then target tier.
        let positions: Vec<usize> = span
            .match_indices("CapabilityTier::")
            .map(|(i, _)| i)
            .collect();
        let [tier_at, target_at] = positions.as_slice() else {
            return Err("an obs() entry must name exactly two tiers".to_owned());
        };
        let ident_end = |at: usize| {
            at + "CapabilityTier::".len()
                + span[at + "CapabilityTier::".len()..]
                    .find(|c: char| !c.is_alphanumeric())
                    .unwrap_or(0)
        };
        let mut out = String::new();
        out.push_str(&span[..*tier_at]);
        out.push_str(&format!(
            "CapabilityTier::{}",
            tier.unwrap_or_else(|| &span[tier_at + 16..ident_end(*tier_at)])
        ));
        out.push_str(&span[ident_end(*tier_at)..*target_at]);
        out.push_str(&format!("CapabilityTier::{target}"));
        out.push_str(&span[ident_end(*target_at)..]);
        return Ok(out);
    }
    let mut out = span.to_owned();
    for (field, value) in [
        ("        tier: CapabilityTier::", tier),
        ("        target_tier: CapabilityTier::", Some(target)),
    ] {
        let Some(value) = value else { continue };
        let at = out
            .find(field)
            .ok_or_else(|| format!("missing {}", field.trim()))?;
        let start = at + field.len();
        let end = start + out[start..].find(',').ok_or("malformed tier field")?;
        out.replace_range(start..end, value);
    }
    Ok(out)
}

/// Applies the manifest policy to the registry source: capabilities listed as supported become
/// `Supported` (tier and target), those listed experimental target `Experimental`. Everything
/// else is untouched. `Err` names what could not be found, and nothing is written.
pub fn apply_policy(
    source: &str,
    supported: &[String],
    experimental: &[String],
) -> Result<String, String> {
    let mut out = source.to_owned();
    for (ids, tier, target) in [
        (supported, Some("Supported"), "Supported"),
        (experimental, None, "Experimental"),
    ] {
        for id in ids {
            let (start, end, obs_call) = definition(&out, id)
                .ok_or_else(|| format!("capability {id} is not in the registry"))?;
            let replaced = set_tiers(&out[start..end], obs_call, tier, target)?;
            out.replace_range(start..end, &replaced);
        }
    }
    Ok(out)
}

/// The documentation block between the campaign markers, replaced in place when present.
pub fn policy_section(
    supported: &[String],
    experimental: &[String],
    internal: &[String],
) -> String {
    let list = |ids: &[String]| {
        ids.iter()
            .map(|i| format!("`{i}`"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "{START}\n## v0.1.0 tier policy\n\nThe validation campaign applies one fixed, conservative policy once live observation, game-runnable, kernel, overhead, corpus, calibration and score evidence all verify:\n\n- **Supported:** {}\n- **Experimental:** {}\n- **Internal:** {}\n\nDiagnosis stays Experimental even after calibration. CPU affinity and cgroup placement are Supported only for their graduated scheduler-pressure tuples (see the autonomous support matrix above); no other mutating capability is promoted.\n{END}\n",
        list(supported),
        list(experimental),
        list(internal)
    )
}

pub fn replace_marked(doc: &str, section: &str) -> String {
    match (doc.find(START), doc.find(END)) {
        (Some(a), Some(b)) if a < b => format!(
            "{}{}{}",
            &doc[..a],
            section.trim_end(),
            &doc[b + END.len()..]
        ),
        _ => format!("{}\n\n{section}", doc.trim_end()),
    }
}

pub struct CapabilityPolicyRunner {
    pub tools: LandingTools,
}

fn blocked(detail: String) -> RunnerOutcome {
    RunnerOutcome::Blocked(BlockReason {
        class: FailureClass::BlockedResource,
        detail,
    })
}

/// Gates in `REQUIRED_GATES` that are not yet satisfied, in their fixed order.
pub fn unmet_gates(context: &RunContext<'_>) -> Vec<String> {
    let Ok(graph) = super::graph::Graph::core() else {
        return REQUIRED_GATES
            .iter()
            .map(|gate| (*gate).to_owned())
            .collect();
    };
    validation_gates()
        .into_iter()
        .filter(|gate| REQUIRED_GATES.contains(&gate.name.as_str()))
        .filter(|gate| {
            gate.tasks.iter().any(|task| {
                graph
                    .get(task)
                    .is_none_or(|spec| !context.state.effective_status(spec).satisfies_dependents())
            })
        })
        .map(|gate| gate.name)
        .collect()
}

fn edit(
    repository: &Path,
    relative: &str,
    change: &dyn Fn(&str) -> Result<String, String>,
) -> Result<(), String> {
    let path = repository.join(relative);
    let current = fs::read_to_string(&path).map_err(|e| format!("{relative}: {e}"))?;
    let updated = change(&current)?;
    if updated != current {
        fs::write(&path, updated).map_err(|e| format!("{relative}: {e}"))?;
    }
    Ok(())
}

impl TaskRunner for CapabilityPolicyRunner {
    fn run(&self, context: &RunContext<'_>, spec: &TaskSpec, attempt: u32) -> RunnerOutcome {
        let unmet = unmet_gates(context);
        if !unmet.is_empty() {
            return blocked(format!(
                "capability tiers cannot change until the evidence for the {unmet:?} gates verifies"
            ));
        }
        let policy = &context.manifest.policy.capabilities;
        let apply = |repository: &Path| -> Result<(), String> {
            edit(repository, REGISTRY, &|src| {
                apply_policy(src, &policy.supported, &policy.experimental)
            })?;
            edit(repository, CAPABILITIES_DOC, &|doc| {
                Ok(replace_marked(
                    doc,
                    &policy_section(&policy.supported, &policy.experimental, &policy.internal),
                ))
            })?;
            edit(repository, LEDGER, &|ledger| {
                let heading = format!(
                    "## Campaign {}: capability policy",
                    context.state.campaign_id
                );
                if ledger.contains(&heading) {
                    return Ok(ledger.to_owned());
                }
                Ok(format!(
                    "{}\n\n{heading}\n\nApplied the fixed v0.1.0 tier policy: Supported {}; every other capability keeps its tier. Calibration and score decisions retained the current thresholds and weights.\n",
                    ledger.trim_end(),
                    policy.supported.join(", ")
                ))
            })
        };
        let landed = self.tools.land(
            context,
            spec,
            attempt,
            &format!(
                "feat(capabilities): apply the v0.1.0 tier policy (campaign {})",
                context.state.campaign_id
            ),
            &apply,
        );
        match landed {
            Ok(_) => RunnerOutcome::passed(),
            Err(outcome) => outcome,
        }
    }
}

/// `docs/CALIBRATION_RESULTS.md` text from the campaign's `calibration/review.json`.
pub fn calibration_doc(review: &Value, campaign: &str) -> String {
    let mut text = format!(
        "# Calibration results\n\nCampaign `{campaign}`. Policy: {}.\n\n| family | decision |\n| --- | --- |\n",
        review["policy"].as_str().unwrap_or("")
    );
    if let Some(families) = review["families"].as_object() {
        for (name, family) in families {
            text.push_str(&format!(
                "| {name} | {} |\n",
                family["decision"].as_str().unwrap_or("?")
            ));
        }
    }
    text.push_str("\n## Limitations\n\n");
    for limit in review["limitations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        text.push_str(&format!("- {limit}\n"));
    }
    text.push_str(&format!(
        "\nDiagnosis capabilities: {}.\n",
        review["diagnosis_capabilities"].as_str().unwrap_or("")
    ));
    text
}

pub fn score_doc(review: &Value, campaign: &str) -> String {
    let mut text = format!(
        "# StutterScore evaluation results\n\nCampaign `{campaign}`. Weights: {}.\n\n- runnable series monotonic: {}\n- frame series monotonic: {}\n\n## Limitations\n\n",
        review["weights"].as_str().unwrap_or(""),
        review["runnable_monotonic"],
        review["frame_monotonic"]
    );
    for limit in review["limitations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        text.push_str(&format!("- {limit}\n"));
    }
    text
}

pub struct ReleaseDocsRunner {
    pub tools: LandingTools,
}

impl TaskRunner for ReleaseDocsRunner {
    fn run(&self, context: &RunContext<'_>, spec: &TaskSpec, attempt: u32) -> RunnerOutcome {
        let read = |relative: &str| -> Option<Value> {
            serde_json::from_slice(&fs::read(context.dir.0.join(relative)).ok()?).ok()
        };
        let (Some(calibration), Some(score)) =
            (read("calibration/review.json"), read("score/review.json"))
        else {
            return blocked("the calibration or score review is missing".to_owned());
        };
        let campaign = context.state.campaign_id.clone();
        let apply = |repository: &Path| -> Result<(), String> {
            fs::write(
                repository.join("docs/CALIBRATION_RESULTS.md"),
                calibration_doc(&calibration, &campaign),
            )
            .map_err(|e| e.to_string())?;
            fs::write(
                repository.join("docs/SCORE_EVALUATION_RESULTS.md"),
                score_doc(&score, &campaign),
            )
            .map_err(|e| e.to_string())?;
            edit(repository, "docs/CORE_VALIDATION_PLAN.md", &|plan| {
                let section = format!(
                    "{START}\n## Campaign result\n\nCampaign `{campaign}` retained the current diagnosis thresholds and StutterScore weights; see `docs/CALIBRATION_RESULTS.md` and `docs/SCORE_EVALUATION_RESULTS.md`. Remaining explicit decisions are recorded as limitations there.\n{END}\n"
                );
                Ok(replace_marked(plan, &section))
            })
        };
        match self.tools.land(
            context,
            spec,
            attempt,
            &format!(
                "docs(validation): record campaign {campaign} calibration and score decisions"
            ),
            &apply,
        ) {
            Ok(_) => RunnerOutcome::passed(),
            Err(outcome) => outcome,
        }
    }
}
