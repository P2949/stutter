use super::*;

#[derive(Args, Debug, Clone)]
pub(super) struct ValidateArgs {
    #[arg(help = "Path to run directory or session.json")]
    pub(super) path: PathBuf,

    #[arg(long)]
    pub(super) json: bool,

    #[arg(long, help = "Treat warnings and medium-quality data as failures")]
    pub(super) strict: bool,
}

pub(super) fn validate_pids(flag: &str, pids: &[u32]) -> anyhow::Result<()> {
    if pids.contains(&0) {
        anyhow::bail!("{flag} must be greater than zero");
    }
    Ok(())
}

pub(super) fn validate_comm_patterns(flag: &str, patterns: &[String]) -> anyhow::Result<()> {
    for pattern in patterns {
        if pattern.is_empty() {
            anyhow::bail!("{flag} patterns must not be empty");
        }
    }
    Ok(())
}

pub(super) fn parse_optional_task_class(value: Option<&str>) -> anyhow::Result<Option<TaskClass>> {
    value
        .map(|s| {
            TaskClass::from_str_opt(s).ok_or_else(|| anyhow::anyhow!("unknown task class: {s}"))
        })
        .transpose()
}

#[cfg(test)]
#[path = "tests/validate.rs"]
mod tests;
