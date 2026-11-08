//! The representative train, build, profile, rebuild, verify and compare workflow as a typed list of
//! steps, and a runner that executes them in order under the process supervisor.
//!
//! A step is an argv, never a shell string: no step can expand a glob, chain a command or redirect.
//! The runner refuses to start unless `validate` has no findings, stops at the first step that does
//! not exit zero, writes every output beneath one work directory, and only ever produces the plan's
//! separate output binary. It never installs or replaces an executable.
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use anyhow::{Context, bail, ensure};
use serde::{Deserialize, Serialize};

use super::{Kind, Plan};
use crate::process::supervisor::{ExecutionPolicy, Outcome, run};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub name: String,
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepResult {
    Passed,
    /// The step ran and exited non-zero, was killed, or timed out.
    Failed(String),
    /// An earlier step failed, so this one never ran.
    NotRun,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRecord {
    pub steps: Vec<(String, StepResult)>,
    pub output_binary: PathBuf,
}

impl RunRecord {
    pub fn all_passed(&self) -> bool {
        self.steps
            .iter()
            .all(|(_, result)| *result == StepResult::Passed)
    }
}

fn substitute(template: &[String], pairs: &[(&str, &str)]) -> Vec<String> {
    template
        .iter()
        .map(|part| {
            pairs
                .iter()
                .fold(part.clone(), |text, (key, value)| text.replace(key, value))
        })
        .collect()
}

fn plain(text: &str) -> bool {
    !text.is_empty()
        && !text
            .chars()
            .any(|c| c.is_whitespace() || c == '\'' || c == '"')
}

/// The ordered steps for `plan`, with every artifact under `work`.
pub fn workflow_steps(plan: &Plan, work: &Path) -> anyhow::Result<Vec<Step>> {
    ensure!(!plan.build_argv.is_empty(), "the plan has no build_argv");
    ensure!(!plan.verify_argv.is_empty(), "the plan has no verify_argv");
    let w = |name: &str| work.join(name).to_string_lossy().into_owned();
    let output = plan.output_binary.to_string_lossy().into_owned();
    let input = plan.input_binary.to_string_lossy().into_owned();
    let clang = plan.compiler.contains("clang");
    let build = |target: &str, extra: Vec<String>| {
        let mut argv = substitute(&plan.build_argv, &[("{output}", target)]);
        argv.extend(plan.flags.iter().cloned());
        argv.extend(extra);
        argv
    };
    let step = |name: &str, argv: Vec<String>, env: Vec<(String, String)>| Step {
        name: name.to_owned(),
        argv,
        env,
    };
    let mut steps = Vec::new();
    // The profile the conversion step reads: imported, or collected here with `perf record`.
    let profile_path = |steps: &mut Vec<Step>, perf_flags: &[&str]| -> anyhow::Result<String> {
        if plan.collect_profile {
            ensure!(
                !plan.run_argv.is_empty(),
                "collecting a profile needs a run_argv"
            );
            let training = plan
                .training_inputs
                .first()
                .context("collecting a profile needs a training input")?;
            let mut argv = vec!["perf".to_owned(), "record".to_owned()];
            argv.extend(perf_flags.iter().map(|flag| (*flag).to_owned()));
            argv.extend(["-o".to_owned(), w("perf.data"), "--".to_owned()]);
            argv.extend(substitute(
                &plan.run_argv,
                &[("{binary}", &input), ("{input}", &training.path)],
            ));
            steps.push(step("collect-profile", argv, Vec::new()));
            Ok(w("perf.data"))
        } else {
            Ok(plan
                .profile
                .as_ref()
                .context("a profile is required")?
                .path
                .clone())
        }
    };
    match plan.kind {
        Kind::Pgo => {
            ensure!(
                !plan.run_argv.is_empty(),
                "PGO needs a run_argv for training"
            );
            let raw = w("raw");
            steps.push(step(
                "build-instrumented",
                build(
                    &w("instrumented"),
                    vec![format!("-fprofile-generate={raw}")],
                ),
                Vec::new(),
            ));
            for (index, training) in plan.training_inputs.iter().enumerate() {
                steps.push(step(
                    &format!("train-{index}"),
                    substitute(
                        &plan.run_argv,
                        &[
                            ("{binary}", &w("instrumented")),
                            ("{input}", &training.path),
                        ],
                    ),
                    vec![("LLVM_PROFILE_FILE".to_owned(), format!("{raw}/%p.profraw"))],
                ));
            }
            let profile = if clang {
                steps.push(step(
                    "merge-profile",
                    vec![
                        "llvm-profdata".to_owned(),
                        "merge".to_owned(),
                        "-o".to_owned(),
                        w("merged.profdata"),
                        raw,
                    ],
                    Vec::new(),
                ));
                w("merged.profdata")
            } else {
                raw
            };
            steps.push(step(
                "rebuild",
                build(&output, vec![format!("-fprofile-use={profile}")]),
                Vec::new(),
            ));
        }
        Kind::AutoFdo => {
            let profile = profile_path(&mut steps, &["-b"])?;
            steps.push(step(
                "convert-profile",
                vec![
                    "create_llvm_prof".to_owned(),
                    format!("--binary={input}"),
                    format!("--profile={profile}"),
                    format!("--out={}", w("autofdo.prof")),
                ],
                Vec::new(),
            ));
            steps.push(step(
                "rebuild",
                build(
                    &output,
                    vec![format!("-fprofile-sample-use={}", w("autofdo.prof"))],
                ),
                Vec::new(),
            ));
        }
        Kind::Bolt => {
            let profile = profile_path(&mut steps, &["-e", "cycles:u", "-j", "any,u"])?;
            steps.push(step(
                "convert-profile",
                vec![
                    "perf2bolt".to_owned(),
                    input.clone(),
                    "-p".to_owned(),
                    profile.clone(),
                    "-o".to_owned(),
                    w("bolt.fdata"),
                ],
                Vec::new(),
            ));
            steps.push(step(
                "bolt",
                vec![
                    "llvm-bolt".to_owned(),
                    input.clone(),
                    "-o".to_owned(),
                    output.clone(),
                    format!("-data={}", w("bolt.fdata")),
                    "-reorder-blocks=ext-tsp".to_owned(),
                    "-reorder-functions=hfsort".to_owned(),
                ],
                Vec::new(),
            ));
        }
    }
    steps.push(step(
        "verify",
        substitute(&plan.verify_argv, &[("{binary}", &output)]),
        Vec::new(),
    ));
    for (index, test) in plan.test_inputs.iter().enumerate() {
        ensure!(
            plain(&test.path) && plain(&input) && plain(&output),
            "a path used in a hyperfine command line must be plain"
        );
        ensure!(!plan.run_argv.is_empty(), "the comparison needs a run_argv");
        let command = |binary: &str| {
            substitute(
                &plan.run_argv,
                &[("{binary}", binary), ("{input}", &test.path)],
            )
            .join(" ")
        };
        steps.push(step(
            &format!("compare-{index}"),
            vec![
                "hyperfine".to_owned(),
                "--export-json".to_owned(),
                w(&format!("ab-{index}.json")),
                command(&input),
                command(&output),
            ],
            Vec::new(),
        ));
    }
    Ok(steps)
}

/// Runs `steps` in order with `timeout` each, stopping at the first failure.
pub fn run_steps(
    steps: &[Step],
    work: &Path,
    output_binary: &Path,
    timeout: Duration,
) -> anyhow::Result<RunRecord> {
    std::fs::create_dir_all(work).context("cannot create the work directory")?;
    let mut record = RunRecord {
        steps: steps
            .iter()
            .map(|step| (step.name.clone(), StepResult::NotRun))
            .collect(),
        output_binary: output_binary.to_path_buf(),
    };
    for (index, step) in steps.iter().enumerate() {
        let Some((program, args)) = step.argv.split_first() else {
            bail!("step {} has an empty argv", step.name);
        };
        let mut command = Command::new(program);
        command.args(args).current_dir(work);
        for (key, value) in &step.env {
            command.env(key, value);
        }
        let mut policy = ExecutionPolicy::bounded(timeout);
        policy.tee = false;
        let result = match run(command, &policy) {
            Ok(done) => match done.outcome {
                Outcome::Exited(status) if status.success() => StepResult::Passed,
                Outcome::Exited(status) => StepResult::Failed(format!("exited with {status}")),
                Outcome::TimedOut => StepResult::Failed("timed out".to_owned()),
                Outcome::Cancelled => StepResult::Failed("cancelled".to_owned()),
            },
            Err(error) => StepResult::Failed(format!("could not start: {error}")),
        };
        let failed = result != StepResult::Passed;
        record.steps[index].1 = result;
        if failed {
            break;
        }
    }
    Ok(record)
}
