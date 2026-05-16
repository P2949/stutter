use super::*;

#[derive(Args, Debug, Clone)]
pub struct ValidateArgs {
    #[arg(help = "Path to run directory or session.json")]
    pub path: PathBuf,

    #[arg(long)]
    pub json: bool,

    #[arg(long, help = "Treat warnings and medium-quality data as failures")]
    pub strict: bool,
}

pub fn validate_pids(flag: &str, pids: &[u32]) -> anyhow::Result<()> {
    if pids.contains(&0) {
        anyhow::bail!("{flag} must be greater than zero");
    }
    Ok(())
}

pub fn validate_comm_patterns(flag: &str, patterns: &[String]) -> anyhow::Result<()> {
    for pattern in patterns {
        if pattern.is_empty() {
            anyhow::bail!("{flag} patterns must not be empty");
        }
    }
    Ok(())
}

pub fn parse_optional_task_class(value: Option<&str>) -> anyhow::Result<Option<TaskClass>> {
    value
        .map(|s| {
            TaskClass::from_str_opt(s).ok_or_else(|| anyhow::anyhow!("unknown task class: {s}"))
        })
        .transpose()
}
