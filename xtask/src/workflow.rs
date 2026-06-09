use std::path::Path;

use anyhow::Context;

use crate::process::run_process;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandSpec {
    pub program: &'static str,
    pub args: &'static [&'static str],
}

#[derive(Clone, Copy, Debug)]
pub struct WorkflowSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub affected_paths: &'static [&'static str],
    pub commands: &'static [CommandSpec],
}

pub fn run_workflow(root: &Path, workflow: WorkflowSpec) -> anyhow::Result<()> {
    print_workflow_header(workflow);
    run_command_specs(root, workflow.commands).with_context(|| workflow_failure_message(workflow))
}

pub fn print_workflow_header(workflow: WorkflowSpec) {
    println!("xtask {}: {}", workflow.name, workflow.description);
    println!("xtask {} affected paths:", workflow.name);
    for path in workflow.affected_paths {
        println!("  - {path}");
    }
}

pub fn workflow_failure_message(workflow: WorkflowSpec) -> String {
    format!(
        "xtask {} failed while processing affected paths: {}",
        workflow.name,
        workflow.affected_paths.join(", ")
    )
}

pub fn run_command_specs(root: &Path, commands: &[CommandSpec]) -> anyhow::Result<()> {
    for command in commands {
        run_process(root, command.program, command.args)?;
    }
    Ok(())
}
