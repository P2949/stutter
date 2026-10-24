//! Deterministic workloads and the tools they need. Everything here builds *commands* and
//! parses *output*; spawning goes through [`ToolHost`] so each driver is testable with a fake
//! backend. Runtime tools come from the pinned Nix environment: nothing is downloaded during
//! evidence collection, and every tool's identity is recorded.
pub mod fio;
pub mod fixed_compile;
pub mod network_irq;
pub mod vkmark;

use std::path::PathBuf;

use serde::Serialize;

use crate::evidence_bundle::sha256::sha256_hex;

/// A command to run: program, arguments, environment additions and working directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: Option<PathBuf>,
}

impl CommandSpec {
    pub fn new(program: &str, args: &[&str]) -> Self {
        Self {
            program: program.to_owned(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            env: Vec::new(),
            cwd: None,
        }
    }

    pub fn with_env(mut self, key: &str, value: &str) -> Self {
        self.env.push((key.to_owned(), value.to_owned()));
        self
    }

    /// Digest of the program and arguments only: a stable identity that carries no paths of
    /// the local machine beyond what the command line itself names.
    pub fn digest(&self) -> String {
        sha256_hex(
            std::iter::once(self.program.as_str())
                .chain(self.args.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join("\u{1f}")
                .as_bytes(),
        )
    }
}

/// What a driver needs from the machine; faked in tests.
pub trait ToolHost {
    fn find(&self, tool: &str) -> Option<PathBuf>;
    /// Runs `spec` to completion and returns stdout (`Err` on launch failure or non-zero exit).
    fn output(&self, spec: &CommandSpec) -> Result<String, String>;
    fn env_var(&self, name: &str) -> Option<String>;
    fn read_file(&self, path: &str) -> Result<String, String>;
    /// Digest of an executable's bytes; `None` when it cannot be read.
    fn executable_sha256(&self, _path: &std::path::Path) -> Option<String> {
        None
    }
}

pub struct SystemTools;

impl ToolHost for SystemTools {
    fn find(&self, tool: &str) -> Option<PathBuf> {
        std::env::var_os("PATH").and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|dir| dir.join(tool))
                .find(|candidate| candidate.is_file())
        })
    }

    fn output(&self, spec: &CommandSpec) -> Result<String, String> {
        let mut command = std::process::Command::new(&spec.program);
        command.args(&spec.args).envs(spec.env.iter().cloned());
        if let Some(cwd) = &spec.cwd {
            command.current_dir(cwd);
        }
        let output = command
            .output()
            .map_err(|e| format!("could not run {}: {e}", spec.program))?;
        if !output.status.success() {
            return Err(format!("{} exited {}", spec.program, output.status));
        }
        String::from_utf8(output.stdout).map_err(|e| format!("{} output: {e}", spec.program))
    }

    fn env_var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok().filter(|value| !value.is_empty())
    }

    fn read_file(&self, path: &str) -> Result<String, String> {
        std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))
    }

    fn executable_sha256(&self, path: &std::path::Path) -> Option<String> {
        std::fs::read(path).ok().map(|bytes| sha256_hex(&bytes))
    }
}

/// Identity of a runtime tool, recorded so a result names exactly what produced it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ToolIdentity {
    pub name: String,
    pub version: String,
    /// Digest of the executable (the path itself is not retained).
    pub sha256: String,
}

/// The tools the campaign needs, with the arguments that print their version.
pub const RUNTIME_TOOLS: [(&str, &[&str]); 7] = [
    ("vkmark", &["--help"]),
    ("mangohud", &["--version"]),
    ("fio", &["--version"]),
    ("stress-ng", &["--version"]),
    ("ping", &["-V"]),
    ("gh", &["--version"]),
    ("taskset", &["--version"]),
];

/// Identifies `tool`; `Err` names exactly what is missing or unusable.
pub fn identify(
    host: &dyn ToolHost,
    tool: &str,
    version_args: &[&str],
) -> Result<ToolIdentity, String> {
    let path = host.find(tool).ok_or_else(|| {
        format!("{tool} is not on PATH (it must come from the pinned Nix environment)")
    })?;
    let version = host
        .output(&CommandSpec::new(tool, version_args))
        .map_err(|error| format!("{tool} did not report a version: {error}"))?
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_owned();
    let sha256 = host.executable_sha256(&path).unwrap_or_default();
    Ok(ToolIdentity {
        name: tool.to_owned(),
        version,
        sha256,
    })
}

/// Identifies every named tool; all failures are reported together.
pub fn preflight_tools(
    host: &dyn ToolHost,
    wanted: &[&str],
) -> Result<Vec<ToolIdentity>, Vec<String>> {
    let (mut found, mut problems) = (Vec::new(), Vec::new());
    for tool in wanted {
        let Some((_, args)) = RUNTIME_TOOLS.iter().find(|(name, _)| name == tool) else {
            problems.push(format!("{tool} is not a known campaign tool"));
            continue;
        };
        match identify(host, tool, args) {
            Ok(identity) => found.push(identity),
            Err(problem) => problems.push(problem),
        }
    }
    if problems.is_empty() {
        Ok(found)
    } else {
        Err(problems)
    }
}
