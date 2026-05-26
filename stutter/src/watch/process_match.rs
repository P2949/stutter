use std::path::{Path, PathBuf};

use stutter_core::ids::Pid;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessMatchDecision {
    pub pid: Pid,
    pub score: u32,
    pub reasons: Vec<ProcessMatchReason>,
}

impl ProcessMatchDecision {
    pub fn reason_labels(&self) -> Vec<&'static str> {
        self.reasons.iter().map(|reason| reason.as_str()).collect()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessMatchReason {
    ExactComm,
    CaseInsensitiveComm,
    ExecutableBasename,
    CommContains,
    CmdlineContains,
}

impl ProcessMatchReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExactComm => "exact_comm",
            Self::CaseInsensitiveComm => "case_insensitive_comm",
            Self::ExecutableBasename => "executable_basename",
            Self::CommContains => "comm_contains",
            Self::CmdlineContains => "cmdline_contains",
        }
    }
}

#[cfg(test)]
pub fn find_process_by_pattern_at(proc_root: &Path, pattern: &str) -> Option<u32> {
    let mut cache = crate::process_tree::ProcessCache::default();
    find_process_by_pattern_at_with_cache(proc_root, pattern, &mut cache)
}

pub fn find_process_by_pattern_at_with_cache(
    proc_root: &Path,
    pattern: &str,
    cache: &mut crate::process_tree::ProcessCache,
) -> Option<u32> {
    find_process_match_by_pattern_at_with_cache(proc_root, pattern, cache)
        .map(|decision| decision.pid.as_u32())
}

pub fn find_process_match_by_pattern_at_with_cache(
    proc_root: &Path,
    pattern: &str,
    cache: &mut crate::process_tree::ProcessCache,
) -> Option<ProcessMatchDecision> {
    let pattern_lower = normalize_process_match_text(pattern);

    let budget = crate::process_tree::ScanBudget::default_proc_scan();
    let mut budget_report = crate::process_tree::ScanBudgetReport::default();

    crate::process_tree::scan_processes_at(proc_root, cache, &budget, &mut budget_report)
        .into_iter()
        .filter_map(|(pid, process)| {
            process_match_decision(pattern, &pattern_lower, &process.comm, &process.cmdline).map(
                |(score, reasons)| ProcessMatchDecision {
                    pid,
                    score,
                    reasons,
                },
            )
        })
        .max_by_key(|decision| (decision.score, decision.pid.as_u32()))
}

#[cfg(test)]
pub fn process_match_score(
    pattern: &str,
    pattern_lower: &str,
    comm: &str,
    cmdline: &str,
) -> Option<u32> {
    process_match_decision(pattern, pattern_lower, comm, cmdline).map(|(score, _)| score)
}

fn process_match_decision(
    pattern: &str,
    pattern_lower: &str,
    comm: &str,
    cmdline: &str,
) -> Option<(u32, Vec<ProcessMatchReason>)> {
    if comm == pattern {
        return Some((5, vec![ProcessMatchReason::ExactComm]));
    }

    let comm_lower = normalize_process_match_text(comm);
    if comm_lower == pattern_lower {
        return Some((4, vec![ProcessMatchReason::CaseInsensitiveComm]));
    }

    let cmdline_lower = normalize_process_match_text(cmdline);
    let exe_basename_lower = cmdline_executable_basename_lower(cmdline);
    if exe_basename_lower.as_deref() == Some(pattern_lower) {
        return Some((3, vec![ProcessMatchReason::ExecutableBasename]));
    }

    if comm_lower.contains(pattern_lower) {
        return Some((2, vec![ProcessMatchReason::CommContains]));
    }

    if cmdline_lower.contains(pattern_lower) {
        return Some((1, vec![ProcessMatchReason::CmdlineContains]));
    }

    None
}

fn normalize_process_match_text(value: &str) -> String {
    value.replace('\\', "/").to_ascii_lowercase()
}

fn cmdline_executable_basename_lower(cmdline: &str) -> Option<String> {
    let executable = cmdline.split_whitespace().next()?;
    let executable = normalize_process_match_text(executable);

    PathBuf::from(executable)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
}
