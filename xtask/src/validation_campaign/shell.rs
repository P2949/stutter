//! Process execution behind a seam, so every repository/GitHub/release step is tested against a
//! scripted machine and the real implementation is a thin wrapper over `std::process`.
use std::{path::Path, process::Command, time::Duration};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ShellOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl ShellOutput {
    pub fn ok(&self) -> bool {
        self.code == 0
    }
}

pub trait Shell {
    /// Runs `program` in `cwd`. A launch failure is reported as exit code -1.
    fn run(&self, program: &str, args: &[&str], cwd: &Path) -> ShellOutput;
    fn sleep(&self, duration: Duration);
}

pub struct SystemShell;

impl Shell for SystemShell {
    fn run(&self, program: &str, args: &[&str], cwd: &Path) -> ShellOutput {
        match Command::new(program).args(args).current_dir(cwd).output() {
            Ok(output) => ShellOutput {
                code: output.status.code().unwrap_or(-1),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            },
            Err(error) => ShellOutput {
                code: -1,
                stdout: String::new(),
                stderr: format!("could not run {program}: {error}"),
            },
        }
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}
