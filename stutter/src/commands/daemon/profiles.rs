use serde::Serialize;

use crate::daemon::state::{
    DaemonProfileEnvironment, DaemonProfileValidation, DaemonState, DaemonStateSnapshotWriter,
    DaemonWorkloadProfile, default_daemon_state_snapshot_path, load_daemon_state,
};

#[derive(Clone, Debug, Serialize)]
pub struct DaemonProfilesListOutput {
    pub state_path: String,
    pub state_loaded: bool,
    pub profiles: Vec<DaemonWorkloadProfile>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DaemonProfilesForgetOutput {
    pub state_path: String,
    pub dry_run: bool,
    pub before_count: usize,
    pub removed_count: usize,
    pub remaining_count: usize,
    pub removed: Vec<DaemonWorkloadProfile>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DaemonProfilesExplainOutput {
    pub state_path: String,
    pub state_loaded: bool,
    pub current_environment: DaemonProfileEnvironment,
    pub profiles: Vec<DaemonProfileExplanationOutput>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DaemonProfileExplanationOutput {
    pub profile: DaemonWorkloadProfile,
    pub validation: DaemonProfileValidation,
}

pub fn run_profiles_command(
    input: crate::commands::input::DaemonProfilesCommandInput,
) -> anyhow::Result<()> {
    match input {
        crate::commands::input::DaemonProfilesCommandInput::List(input) => {
            run_profiles_list_command(input)
        }
        crate::commands::input::DaemonProfilesCommandInput::Forget(input) => {
            run_profiles_forget_command(input)
        }
        crate::commands::input::DaemonProfilesCommandInput::Explain(input) => {
            run_profiles_explain_command(input)
        }
    }
}

pub fn run_profiles_list_command(
    input: crate::commands::input::DaemonProfilesListCommandInput,
) -> anyhow::Result<()> {
    let output = build_profiles_list_output();

    if input.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print!("{}", render_profiles_list_text(&output));
    }

    Ok(())
}

pub fn run_profiles_forget_command(
    input: crate::commands::input::DaemonProfilesForgetCommandInput,
) -> anyhow::Result<()> {
    let output = forget_daemon_profiles(&input)?;

    if input.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print!("{}", render_profiles_forget_text(&output));
    }

    Ok(())
}

pub fn run_profiles_explain_command(
    input: crate::commands::input::DaemonProfilesExplainCommandInput,
) -> anyhow::Result<()> {
    let output = build_profiles_explain_output(input.workload_identity_hash.as_deref());

    if input.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print!("{}", render_profiles_explain_text(&output));
    }

    Ok(())
}

pub fn build_profiles_list_output() -> DaemonProfilesListOutput {
    let (state_path, state_loaded, state) = load_daemon_state_for_profile_commands();
    build_profiles_list_output_from_state(state_path.display().to_string(), state_loaded, &state)
}

pub fn build_profiles_list_output_from_state(
    state_path: String,
    state_loaded: bool,
    state: &DaemonState,
) -> DaemonProfilesListOutput {
    DaemonProfilesListOutput {
        state_path,
        state_loaded,
        profiles: state.profile_memory.sorted_profiles(),
    }
}

pub fn build_profiles_explain_output(
    workload_identity_hash: Option<&str>,
) -> DaemonProfilesExplainOutput {
    let (state_path, state_loaded, state) = load_daemon_state_for_profile_commands();
    build_profiles_explain_output_from_state(
        state_path.display().to_string(),
        state_loaded,
        &state,
        workload_identity_hash,
        DaemonProfileEnvironment::current(),
        crate::audit::unix_nanos_now(),
    )
}

pub fn build_profiles_explain_output_from_state(
    state_path: String,
    state_loaded: bool,
    state: &DaemonState,
    workload_identity_hash: Option<&str>,
    current_environment: DaemonProfileEnvironment,
    now_unix_nanos: u128,
) -> DaemonProfilesExplainOutput {
    let profiles = state
        .profile_memory
        .sorted_profiles()
        .into_iter()
        .filter(|profile| {
            workload_identity_hash
                .map(|hash| profile.workload_identity_hash == hash)
                .unwrap_or(true)
        })
        .map(|profile| {
            let validation = profile.validation(&current_environment, now_unix_nanos);
            DaemonProfileExplanationOutput {
                profile,
                validation,
            }
        })
        .collect();

    DaemonProfilesExplainOutput {
        state_path,
        state_loaded,
        current_environment,
        profiles,
    }
}

pub fn forget_daemon_profiles(
    input: &crate::commands::input::DaemonProfilesForgetCommandInput,
) -> anyhow::Result<DaemonProfilesForgetOutput> {
    let state_path = default_daemon_state_snapshot_path();
    let mut state = if state_path.exists() {
        load_daemon_state(&state_path)?
    } else {
        DaemonState::default()
    };

    let before_count = state.profile_memory.profiles.len();
    let removed = state.profile_memory.forget_matching(
        input.workload_identity_hash.as_deref(),
        input.candidate.as_deref(),
        input.all,
    );
    let remaining_count = state.profile_memory.profiles.len();

    if !input.dry_run {
        DaemonStateSnapshotWriter::new(&state_path).write(&state)?;
    }

    Ok(DaemonProfilesForgetOutput {
        state_path: state_path.display().to_string(),
        dry_run: input.dry_run,
        before_count,
        removed_count: removed.len(),
        remaining_count,
        removed,
    })
}

pub fn load_daemon_state_for_profile_commands() -> (std::path::PathBuf, bool, DaemonState) {
    let state_path = default_daemon_state_snapshot_path();
    match load_daemon_state(&state_path) {
        Ok(state) => (state_path, true, state),
        Err(err) => {
            log::debug!(
                "daemon_profile_state_load_failed path={} err={err:#}",
                state_path.display()
            );
            (state_path, false, DaemonState::default())
        }
    }
}

pub fn render_profiles_list_text(output: &DaemonProfilesListOutput) -> String {
    let mut text = String::new();
    text.push_str("Daemon profiles\n");
    text.push_str(&format!("state_path: {}\n", output.state_path));
    text.push_str(&format!("state_loaded: {}\n", output.state_loaded));
    text.push_str(&format!("profiles: {}\n", output.profiles.len()));

    if output.profiles.is_empty() {
        text.push_str("profile: none\n");
    } else {
        for profile in &output.profiles {
            text.push_str(&format!(
                "profile workload_hash={} label={} candidate={} action_id={} safety_class={:?} confidence_milli={} kept_unix_nanos={}\n",
                profile.workload_identity_hash,
                profile.workload_label.as_deref().unwrap_or("-"),
                profile.candidate_name,
                profile.action_id,
                profile.safety_class,
                profile.confidence_milli,
                profile.kept_unix_nanos
            ));
        }
    }

    text
}

pub fn render_profiles_forget_text(output: &DaemonProfilesForgetOutput) -> String {
    let mut text = String::new();
    text.push_str("Daemon profiles forget\n");
    text.push_str(&format!("state_path: {}\n", output.state_path));
    text.push_str(&format!("dry_run: {}\n", output.dry_run));
    text.push_str(&format!("before_count: {}\n", output.before_count));
    text.push_str(&format!("removed_count: {}\n", output.removed_count));
    text.push_str(&format!("remaining_count: {}\n", output.remaining_count));
    for profile in &output.removed {
        text.push_str(&format!(
            "removed workload_hash={} candidate={} action_id={}\n",
            profile.workload_identity_hash, profile.candidate_name, profile.action_id
        ));
    }
    text
}

pub fn render_profiles_explain_text(output: &DaemonProfilesExplainOutput) -> String {
    let mut text = String::new();
    text.push_str("Daemon profiles explain\n");
    text.push_str(&format!("state_path: {}\n", output.state_path));
    text.push_str(&format!("state_loaded: {}\n", output.state_loaded));
    text.push_str(&format!("profiles: {}\n", output.profiles.len()));
    text.push_str(&format!(
        "current_kernel: {}\n",
        output
            .current_environment
            .kernel_version
            .as_deref()
            .unwrap_or("-")
    ));
    text.push_str(&format!(
        "current_cpu_topology_hash: {}\n",
        output
            .current_environment
            .cpu_topology_hash
            .as_deref()
            .unwrap_or("-")
    ));

    if output.profiles.is_empty() {
        text.push_str("profile: none\n");
    } else {
        for explanation in &output.profiles {
            let reasons = if explanation.validation.reason_codes.is_empty() {
                "none".to_owned()
            } else {
                explanation.validation.reason_codes.join(",")
            };
            text.push_str(&format!(
                "profile workload_hash={} candidate={} valid={} confidence_milli={} reason_codes={}\n",
                explanation.profile.workload_identity_hash,
                explanation.profile.candidate_name,
                explanation.validation.valid,
                explanation.validation.confidence_milli,
                reasons
            ));
        }
    }

    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::SafetyClass;

    fn daemon_test_profile(workload: &str, candidate: &str) -> DaemonWorkloadProfile {
        DaemonWorkloadProfile {
            workload_identity_hash: workload.to_owned(),
            workload_label: Some("game".to_owned()),
            candidate_name: candidate.to_owned(),
            action_id: format!("cpu-affinity-profile:{candidate}"),
            action_kind: "cpu_affinity_profile".to_owned(),
            safety_class: SafetyClass::ReversibleLowRisk,
            kept_unix_nanos: 1_000,
            last_validated_unix_nanos: Some(1_000),
            baseline_score_total: Some(1_000),
            candidate_score_total: Some(850),
            score_delta: -150,
            confidence_milli: 900,
            environment: DaemonProfileEnvironment {
                hardware_fingerprint: Some("hardware-a".to_owned()),
                kernel_version: Some("6.12.0".to_owned()),
                cpu_topology_hash: Some("topology-a".to_owned()),
                scheduler_label: Some("scx_lavd".to_owned()),
                scx_ops: Some("scx_lavd".to_owned()),
                ..DaemonProfileEnvironment::default()
            },
            partition: crate::daemon::state::DaemonProfilePartition {
                power_source: Some("ac".to_owned()),
                scheduler_label: Some("scx_lavd".to_owned()),
                ..crate::daemon::state::DaemonProfilePartition::default()
            },
        }
    }

    #[test]
    fn daemon_profiles_list_text_reports_persistent_profiles() {
        let state = DaemonState {
            profile_memory: crate::daemon::state::DaemonProfileMemory {
                profiles: vec![daemon_test_profile("workload-a", "game-main")],
            },
            ..DaemonState::default()
        };
        let output = build_profiles_list_output_from_state("state.json".to_owned(), true, &state);

        let text = render_profiles_list_text(&output);

        assert!(text.contains("Daemon profiles"));
        assert!(text.contains("profiles: 1"));
        assert!(text.contains("workload_hash=workload-a"));
        assert!(text.contains("candidate=game-main"));
    }

    #[test]
    fn daemon_profiles_explain_text_reports_invalidation_reasons() {
        let state = DaemonState {
            profile_memory: crate::daemon::state::DaemonProfileMemory {
                profiles: vec![daemon_test_profile("workload-a", "game-main")],
            },
            ..DaemonState::default()
        };
        let current_environment = DaemonProfileEnvironment {
            hardware_fingerprint: Some("hardware-a".to_owned()),
            kernel_version: Some("6.13.0".to_owned()),
            cpu_topology_hash: Some("topology-b".to_owned()),
            scheduler_label: Some("scx_bpfland".to_owned()),
            scx_ops: Some("scx_bpfland".to_owned()),
            ..DaemonProfileEnvironment::default()
        };
        let output = build_profiles_explain_output_from_state(
            "state.json".to_owned(),
            true,
            &state,
            Some("workload-a"),
            current_environment,
            1_000,
        );

        let text = render_profiles_explain_text(&output);

        assert!(text.contains("Daemon profiles explain"));
        assert!(text.contains("valid=false"));
        assert!(text.contains("kernel_changed"));
        assert!(text.contains("cpu_topology_changed"));
    }

    #[test]
    fn daemon_profiles_forget_text_summarizes_removed_profiles() {
        let output = DaemonProfilesForgetOutput {
            state_path: "state.json".to_owned(),
            dry_run: true,
            before_count: 2,
            removed_count: 1,
            remaining_count: 1,
            removed: vec![daemon_test_profile("workload-a", "game-main")],
        };

        let text = render_profiles_forget_text(&output);

        assert!(text.contains("Daemon profiles forget"));
        assert!(text.contains("dry_run: true"));
        assert!(text.contains("removed_count: 1"));
        assert!(text.contains("removed workload_hash=workload-a candidate=game-main"));
    }
}
