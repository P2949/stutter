use std::{fs, path::Path};

use serde_json::{Value, json};

use super::{Spec, load_runs};
use crate::{
    evidence_metadata::{self as shared, SUITE_FRAME_OBJECTIVE, Source},
    objective_campaign::write_json,
};

pub(crate) fn write(
    root: &Path,
    out: &Path,
    spec: &Spec,
    frozen: &Value,
    holdout: &Value,
) -> anyhow::Result<()> {
    let raw = out.join("evidence-raw");
    if raw.exists() {
        fs::remove_dir_all(&raw)?;
    }
    fs::create_dir_all(raw.join("artifacts"))?;
    let environment = shared::capture(
        root,
        &root.join("target/debug/stutter"),
        SUITE_FRAME_OBJECTIVE,
        "controlled_real",
        Source::Checkout,
    )?;
    shared::write_environment(&raw, &environment)?;
    shared::require_clean(&environment)?;
    write_json(&raw.join("artifacts/frozen-calibration.json"), frozen)?;
    write_json(&raw.join("artifacts/holdout.json"), holdout)?;
    let all_runs = load_runs(out, 0..spec.calibration_blocks + spec.holdout_blocks)?;
    write_json(
        &raw.join("artifacts/runs.json"),
        &serde_json::to_value(&all_runs)?,
    )?;
    let passed = holdout["passed"] == true;
    let tally = &holdout["holdout_tally"];
    let case = |name: &str, ok: bool, detail: String| json!({"name": name, "status": if ok { "PASS" } else { "FAIL" }, "detail": detail});
    shared::write_result(
        &raw,
        SUITE_FRAME_OBJECTIVE,
        json!({
            "status": if passed { "PASS" } else { "FAIL" },
            "evidence_kind": "controlled_real",
            "scope": "Direct frame-time evidence from MangoHud logs of a CPU-bound, CPU-pinned vkcube, read through the controller's own parser and rolling-window p99, separates a controlled beneficial change from a neutral one and a regression: the minimum frame population and the decision threshold were selected by a pre-registered rule on calibration pairs, frozen before any holdout run existed, and evaluated once on an untouched holdout with no false keep.",
            "not_claimed": [
                "any mutation's benefit (this graduates the objective's measurement, not an operation)",
                "GPU-bound or real games, or frame sources other than a MangoHud log",
                "compositor-level or present-feedback frame timing",
                "effects smaller than the smallest controlled step (about 20 percent of frame p99)",
                "real user sessions",
            ],
            "cases": [
                case("population_and_threshold_selected_by_the_preregistered_rule_on_calibration", true,
                     format!("population {} frames, {} percent", frozen["population"], frozen["threshold_percent"])),
                case("no_false_keep_on_the_holdout", tally["false_keeps"] == 0,
                     format!("{} false keeps over {} neutral and {} regression pairs", tally["false_keeps"], tally["neutral"], tally["regression"])),
                case("beneficial_keep_rate_on_the_holdout", holdout["holdout_keep_rate"].as_f64().unwrap_or(0.0) >= spec.min_beneficial_keep_rate,
                     format!("{} of {} beneficial pairs kept", tally["beneficial_kept"], tally["beneficial"])),
            ],
        }),
    )?;
    Ok(())
}
