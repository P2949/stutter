use std::path::Path;

use anyhow::{Context, bail};
use stutter::api::autotune::ObjectiveKind;

pub fn render() -> String {
    let mut result = String::from(
        "# Auto-tune Objectives\n\nGenerated from `ObjectiveKind::descriptor()`. Regenerate with\n`cargo run -p xtask -- objective-docs --write`; CI uses `--check`.\n\nSignal quality below describes the declared primary contract. Actual observation\nquality can be lower or missing. Autonomous eligibility remains subject to the\nseparate experimental action, policy and safety gates; it does not enable mode\nor grant permission. No objective has graduated to Supported.\n\n| Objective | Primary metric | Quality | Required signals | Fallback | Guardrails | Autonomous eligible | Maturity |\n| --- | --- | --- | --- | --- | --- | --- | --- |\n",
    );
    for kind in ObjectiveKind::ALL {
        let d = kind.descriptor();
        result.push_str(&format!(
            "| `{kind:?}` | `{}` | {} | {} | {:?} | {} | {} | {:?} |\n",
            d.primary_metric,
            d.primary_signal_quality.as_str(),
            d.required_signals.join(", "),
            d.fallback_policy,
            d.guardrails.join(", "),
            d.autonomous_apply_allowed,
            d.maturity
        ));
    }
    result.push_str("\n## Limitations\n\n");
    for kind in ObjectiveKind::ALL {
        result.push_str(&format!(
            "- `{kind:?}`: {}\n",
            kind.descriptor().limitations
        ));
    }
    result.push_str("\nSee [runnable latency](RUNNABLE_LATENCY_OBJECTIVE.md),\n[fixed-work compile completion](FIXED_WORK_COMPILE.md), and the\n[behavioral matrix](OBJECTIVE_CONTRACT_MATRIX.md) for detailed contracts and\nvalidation boundaries. Historical `compile_progress_*` fields deserialize as\nactivity; legacy Direct labels never authorize a throughput improvement.\n");
    result
}

pub fn run_at(path: &Path, write: bool) -> anyhow::Result<()> {
    let expected = render();
    if write {
        std::fs::write(path, expected).context("write generated objective documentation")?;
    } else {
        let actual = std::fs::read_to_string(path).context("read objective documentation")?;
        if actual != expected {
            bail!(
                "objective documentation is stale: run cargo run -p xtask -- objective-docs --write"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;
    #[test]
    fn documentation_command_requires_exactly_one_mode() {
        assert!(crate::Cli::try_parse_from(["xtask", "objective-docs"]).is_err());
        assert!(
            crate::Cli::try_parse_from(["xtask", "objective-docs", "--check", "--write"]).is_err()
        );
        assert!(crate::Cli::try_parse_from(["xtask", "objective-docs", "--check"]).is_ok());
        assert!(crate::Cli::try_parse_from(["xtask", "objective-docs", "--write"]).is_ok());
    }
    #[test]
    fn documentation_check_rejects_drift_and_write_repairs_it() {
        let path =
            std::env::temp_dir().join(format!("stutter-objective-docs-{}.md", std::process::id()));
        std::fs::write(&path, "outdated contract\n").unwrap();
        assert!(run_at(&path, false).is_err());
        run_at(&path, true).unwrap();
        run_at(&path, false).unwrap();
        let changed = render().replace("runnable_p99_ns", "wrong_metric");
        std::fs::write(&path, changed).unwrap();
        assert!(run_at(&path, false).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
