use std::collections::BTreeSet;

use crate::{
    actions::irq_affinity::{IrqAffinityAction, IrqAffinityEvidence},
    autotune::{
        candidate::{CandidateAction, CandidateEvidence, IrqAffinityActionPlan},
        objective::ObjectiveKind,
        observation::{ActiveConfigSnapshot, ActiveTaskSnapshot},
        providers::{
            CandidateProposal, CandidateProvider, CandidateProviderInput,
            signal_quality_confidence_weight,
        },
        situation::SituationKind,
    },
    irq_inspect::{IrqLine, classify_irq_device, is_numeric_irq},
    process_tree::TaskClass,
};

#[derive(Clone, Debug, PartialEq)]
pub struct IrqCandidateEvidence {
    pub irq: u32,
    pub device: String,
    pub current_mask: String,
    pub suggested_mask: String,
    pub overlap_score: f32,
    pub stable_identity: bool,
    pub known_device_mapping: bool,
    pub placement_rationale: String,
    pub irq_line: IrqLine,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CpuPlacementMap {
    pub focused_workload_cpus: BTreeSet<u32>,
    pub compositor_cpus: BTreeSet<u32>,
    pub audio_realtime_cpus: BTreeSet<u32>,
    pub housekeeping_cpus: BTreeSet<u32>,
    pub reserved_cpus: BTreeSet<u32>,
    pub candidate_irq_cpus: BTreeSet<u32>,
}

#[derive(Default)]
pub struct IrqAffinityProvider;

impl CandidateProvider for IrqAffinityProvider {
    fn family(&self) -> &'static str {
        "irq_affinity"
    }

    fn propose(&self, input: &CandidateProviderInput<'_>) -> Vec<CandidateProposal> {
        if !input.capabilities.irq_affinity_available
            || input.observation.primary_situation != SituationKind::IrqPressure
        {
            return Vec::new();
        }

        let Some(evidence_model) = observed_irq(input) else {
            return Vec::new();
        };

        let device_class = classify_irq_device(&evidence_model.irq_line);
        let evidence = IrqAffinityEvidence {
            strong_irq_evidence: true,
            stable_irq_identity: evidence_model.stable_identity,
            known_device_mapping: evidence_model.known_device_mapping,
            observed_irq: Some(evidence_model.irq),
            observed_device_hint: Some(evidence_model.device.clone()),
            reason: format!(
                "IRQ pressure selected from structured telemetry; current_mask={} suggested_mask={} overlap_score={:.3}; {}",
                evidence_model.current_mask,
                evidence_model.suggested_mask,
                evidence_model.overlap_score,
                evidence_model.placement_rationale,
            ),
        };
        let candidate = CandidateAction::IrqAffinity {
            plan: IrqAffinityActionPlan {
                name: format!("irq-{}-investigate-affinity", evidence_model.irq),
                action: IrqAffinityAction::new(
                    evidence_model.irq,
                    evidence_model.device.clone(),
                    evidence_model.suggested_mask.clone(),
                    device_class.default_risk(),
                    evidence,
                ),
                evidence: vec![CandidateEvidence::new(
                    "irq_structured",
                    format!(
                        "irq={} device={} current_mask={} suggested_mask={} overlap_score={:.3} stable_identity={} known_device_mapping={} placement={}",
                        evidence_model.irq,
                        evidence_model.device,
                        evidence_model.current_mask,
                        evidence_model.suggested_mask,
                        evidence_model.overlap_score,
                        evidence_model.stable_identity,
                        evidence_model.known_device_mapping,
                        evidence_model.placement_rationale,
                    ),
                    irq_confidence(input, &evidence_model),
                )],
                objective: ObjectiveKind::IrqOverlapReduction,
            },
        };

        vec![CandidateProposal {
            candidate,
            provider: self.family(),
            confidence: irq_confidence(input, &evidence_model),
            deny_reasons: Vec::new(),
            objective: ObjectiveKind::IrqOverlapReduction,
            rank_hint: 60,
        }]
    }
}

fn observed_irq(input: &CandidateProviderInput<'_>) -> Option<IrqCandidateEvidence> {
    let signals = &input.observation.objective_signals;
    if !signals.has_irq_signal() {
        return None;
    }

    let irq = signals.irq_hot_irq?;
    let current_mask = input
        .observation
        .active_config_snapshot
        .as_ref()
        .or(Some(&input.system_context.active_config))?
        .irq
        .per_irq
        .get(&irq)?
        .clone();

    let irq_line = input
        .system_context
        .inventory
        .irq_lines
        .iter()
        .find(|line| numeric_irq(line) == Some(irq))?;

    let placement = cpu_placement_map(input, irq_line, signals.irq_hot_cpu);
    let (suggested_cpu, placement_rationale) = select_irq_cpu(
        input,
        irq_line,
        &placement,
        overlap_score(signals.irq_worst_overlap_ns.unwrap_or(0)),
    )?;
    let suggested_mask = single_cpu_mask(suggested_cpu)?;
    if suggested_mask.trim() == current_mask.trim() {
        return None;
    }

    let stable_identity = !irq_line.name.trim().is_empty();
    let known_device_mapping = stable_identity && !irq_line.kind.trim().is_empty();
    if !stable_identity {
        return None;
    }

    Some(IrqCandidateEvidence {
        irq,
        device: irq_device_label(irq_line),
        current_mask,
        suggested_mask,
        overlap_score: overlap_score(signals.irq_worst_overlap_ns.unwrap_or(0)),
        stable_identity,
        known_device_mapping,
        placement_rationale,
        irq_line: irq_line.clone(),
    })
}

fn numeric_irq(line: &IrqLine) -> Option<u32> {
    is_numeric_irq(&line.irq)
        .then(|| line.irq.parse::<u32>().ok())
        .flatten()
}

fn irq_device_label(line: &IrqLine) -> String {
    let name = line.name.trim();
    if name.is_empty() {
        format!("irq-{}", line.irq)
    } else {
        name.to_owned()
    }
}

fn least_busy_cpu_in_set(line: &IrqLine, cpus: &BTreeSet<u32>) -> Option<u32> {
    cpus.iter()
        .filter_map(|cpu| {
            line.counts_by_cpu
                .get(*cpu as usize)
                .map(|count| (*cpu, *count))
        })
        .min_by_key(|(_, count)| *count)
        .map(|(cpu, _)| cpu)
}

fn select_irq_cpu(
    input: &CandidateProviderInput<'_>,
    irq_line: &IrqLine,
    placement: &CpuPlacementMap,
    overlap_score: f32,
) -> Option<(u32, String)> {
    if let Some(cpu) = least_busy_cpu_in_set(irq_line, &placement.housekeeping_cpus) {
        return Some((cpu, placement_rationale("housekeeping", placement)));
    }

    let protected = placement
        .audio_realtime_cpus
        .union(&placement.reserved_cpus)
        .copied()
        .collect::<BTreeSet<_>>();
    let non_focused_candidates = placement
        .candidate_irq_cpus
        .difference(&protected)
        .copied()
        .filter(|cpu| !placement.compositor_cpus.contains(cpu))
        .filter(|cpu| !placement.focused_workload_cpus.contains(cpu))
        .collect::<BTreeSet<_>>();
    if let Some(cpu) = least_busy_cpu_in_set(irq_line, &non_focused_candidates) {
        return Some((cpu, placement_rationale("non_focused_candidate", placement)));
    }

    if irq_belongs_to_focused_device(input, irq_line) && overlap_score >= 0.5 {
        let focused_candidates = placement
            .focused_workload_cpus
            .intersection(&placement.candidate_irq_cpus)
            .copied()
            .filter(|cpu| !protected.contains(cpu))
            .collect::<BTreeSet<_>>();
        if let Some(cpu) = least_busy_cpu_in_set(irq_line, &focused_candidates) {
            return Some((
                cpu,
                placement_rationale("focused_device_exception", placement),
            ));
        }
    }

    None
}

fn cpu_placement_map(
    input: &CandidateProviderInput<'_>,
    irq_line: &IrqLine,
    hot_cpu: Option<u32>,
) -> CpuPlacementMap {
    let active_config = active_config(input);
    let mut map = CpuPlacementMap {
        candidate_irq_cpus: candidate_irq_cpus(input, irq_line, hot_cpu),
        ..CpuPlacementMap::default()
    };

    for task in &input.observation.active_tasks {
        add_task_cpus_to_placement(task, active_config, &mut map);
    }
    for task in input
        .observation
        .protected_tasks
        .iter()
        .map(|task| ActiveTaskSnapshot {
            tid: task.tid,
            process_pid: task.process_pid,
            comm: task.comm.clone(),
            class: task.class,
            process_starttime_ticks: None,
            task_starttime_ticks: None,
            cgroup_path: None,
        })
    {
        add_task_cpus_to_placement(&task, active_config, &mut map);
    }

    let blocked = map
        .focused_workload_cpus
        .union(&map.compositor_cpus)
        .copied()
        .chain(map.audio_realtime_cpus.iter().copied())
        .chain(map.reserved_cpus.iter().copied())
        .collect::<BTreeSet<_>>();
    map.housekeeping_cpus = map
        .candidate_irq_cpus
        .difference(&blocked)
        .copied()
        .collect();

    map
}

fn active_config<'a>(input: &'a CandidateProviderInput<'_>) -> &'a ActiveConfigSnapshot {
    input
        .observation
        .active_config_snapshot
        .as_ref()
        .unwrap_or(&input.system_context.active_config)
}

fn add_task_cpus_to_placement(
    task: &ActiveTaskSnapshot,
    active_config: &ActiveConfigSnapshot,
    map: &mut CpuPlacementMap,
) {
    let cpus = active_config
        .affinity
        .per_tid
        .get(&task.tid)
        .and_then(|value| crate::topology::parse_cpu_list(value).ok())
        .unwrap_or_default()
        .into_iter()
        .collect::<BTreeSet<_>>();
    if cpus.is_empty() {
        return;
    }

    if is_audio_realtime_class(task.class) {
        map.audio_realtime_cpus.extend(cpus);
    } else if is_compositor_class(task.class) {
        map.compositor_cpus.extend(cpus);
    } else if is_focused_workload_class(task.class) {
        map.focused_workload_cpus.extend(cpus);
    } else if is_reserved_class(task.class) {
        map.reserved_cpus.extend(cpus);
    }
}

fn candidate_irq_cpus(
    input: &CandidateProviderInput<'_>,
    irq_line: &IrqLine,
    hot_cpu: Option<u32>,
) -> BTreeSet<u32> {
    if let Some(mask) = input
        .system_context
        .inventory
        .irq_default_smp_affinity
        .as_deref()
        && let Some(cpus) = cpus_from_irq_hex_mask(mask)
        && !cpus.is_empty()
    {
        return cpus;
    }

    let from_counts = (0..irq_line.counts_by_cpu.len() as u32).collect::<BTreeSet<_>>();
    if !from_counts.is_empty() {
        return from_counts;
    }

    hot_cpu.into_iter().collect()
}

fn cpus_from_irq_hex_mask(mask: &str) -> Option<BTreeSet<u32>> {
    let compact = mask
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<String>();
    let bits = u128::from_str_radix(&compact, 16).ok()?;
    Some(
        (0..128)
            .filter(|cpu| bits & (1_u128 << cpu) != 0)
            .map(|cpu| cpu as u32)
            .collect(),
    )
}

fn placement_rationale(selection: &str, placement: &CpuPlacementMap) -> String {
    format!(
        "selection={} candidate_irq_cpus={} housekeeping_cpus={} focused_workload_cpus={} compositor_cpus={} audio_realtime_cpus={} reserved_cpus={}",
        selection,
        format_cpu_set(&placement.candidate_irq_cpus),
        format_cpu_set(&placement.housekeeping_cpus),
        format_cpu_set(&placement.focused_workload_cpus),
        format_cpu_set(&placement.compositor_cpus),
        format_cpu_set(&placement.audio_realtime_cpus),
        format_cpu_set(&placement.reserved_cpus),
    )
}

fn format_cpu_set(cpus: &BTreeSet<u32>) -> String {
    if cpus.is_empty() {
        "{}".to_owned()
    } else {
        format!(
            "{{{}}}",
            cpus.iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(",")
        )
    }
}

fn irq_belongs_to_focused_device(input: &CandidateProviderInput<'_>, irq_line: &IrqLine) -> bool {
    let label = irq_device_label(irq_line).to_ascii_lowercase();
    let gpu_like = label.contains("gpu")
        || label.contains("amdgpu")
        || label.contains("nvidia")
        || label.contains("i915");
    gpu_like
        && matches!(
            input.observation.focus_kind,
            Some(crate::focus::FocusGroupKind::Game)
        )
}

fn is_audio_realtime_class(class: TaskClass) -> bool {
    matches!(class, TaskClass::AudioRealtime | TaskClass::Input)
}

fn is_compositor_class(class: TaskClass) -> bool {
    matches!(class, TaskClass::Compositor | TaskClass::GameScope)
}

fn is_focused_workload_class(class: TaskClass) -> bool {
    matches!(
        class,
        TaskClass::Game
            | TaskClass::GameHelper
            | TaskClass::GameRenderThread
            | TaskClass::GameWorkerThread
            | TaskClass::WineServer
            | TaskClass::SteamRuntime
            | TaskClass::BrowserForeground
            | TaskClass::BrowserRenderer
            | TaskClass::BrowserGpu
            | TaskClass::Render
    )
}

fn is_reserved_class(class: TaskClass) -> bool {
    matches!(
        class,
        TaskClass::KernelThread
            | TaskClass::IrqThread
            | TaskClass::Service
            | TaskClass::NetworkDaemon
            | TaskClass::StorageDaemon
            | TaskClass::Unknown
    )
}

fn single_cpu_mask(cpu: u32) -> Option<String> {
    1_u128.checked_shl(cpu).map(|mask| format!("{mask:x}"))
}

fn overlap_score(worst_overlap_ns: u64) -> f32 {
    ((worst_overlap_ns as f32) / 5_000_000.0).clamp(0.0, 1.0)
}

fn irq_confidence(input: &CandidateProviderInput<'_>, evidence: &IrqCandidateEvidence) -> f32 {
    let completeness = [
        true,
        evidence.stable_identity,
        evidence.known_device_mapping,
        !evidence.current_mask.is_empty(),
        !evidence.suggested_mask.is_empty(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count() as f32
        / 5.0;

    let signal_weight = signal_quality_confidence_weight(
        input
            .observation
            .objective_signals
            .signal_quality
            .irq_overlap,
    );

    (input.observation.situation.confidence * completeness * signal_weight).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::{
        autotune::{
            controller::ControllerRuntimeState,
            observation::{
                ActiveAffinitySnapshot, ActiveConfigSnapshot, ActiveIrqSnapshot,
                ActiveTaskSnapshot, AutotuneObservation,
            },
            system_context::SystemContextSnapshot,
        },
        daemon::{
            capabilities::DaemonCapabilities,
            health::SystemHealthSnapshot,
            policy::{ActionSource, DaemonMode},
        },
        daemon_policy::{DaemonPolicyBuildInput, build_daemon_policy},
        focus::FocusGroupKind,
        system_inventory::SystemInventory,
    };

    #[test]
    fn irq_provider_selects_hot_irq_from_structured_telemetry() {
        let provider = IrqAffinityProvider;
        let mut observation = observation();
        observation.objective_signals.irq_overlap_count = Some(2);
        observation.objective_signals.irq_worst_overlap_ns = Some(4_000_000);
        observation.objective_signals.irq_hot_irq = Some(146);
        observation.objective_signals.irq_hot_cpu = Some(2);
        observation.active_config_snapshot = Some(ActiveConfigSnapshot {
            irq: ActiveIrqSnapshot {
                per_irq: BTreeMap::from([(146, "4".to_owned())]),
            },
            ..ActiveConfigSnapshot::default()
        });

        let mut system_context = system_context();
        system_context.inventory.irq_default_smp_affinity = Some("f".to_owned());
        system_context.inventory.irq_lines = vec![IrqLine {
            irq: "146".to_owned(),
            counts_by_cpu: vec![10_000, 2, 20_000, 30_000],
            total: 60_002,
            kind: "PCI-MSI".to_owned(),
            name: "amdgpu".to_owned(),
            raw: "146: 10000 2 20000 30000 PCI-MSI amdgpu".to_owned(),
        }];

        let proposals = provider.propose(&CandidateProviderInput {
            observation: &observation,
            daemon_policy: &policy(),
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            system_context: &system_context,
            controller_state: &ControllerRuntimeState::default(),
            profiles: &[],
        });

        assert_eq!(proposals.len(), 1);
        let CandidateAction::IrqAffinity { plan } = &proposals[0].candidate else {
            panic!("expected irq affinity candidate");
        };
        assert_eq!(plan.action.irq, 146);
        assert_eq!(plan.action.device_hint, "amdgpu");
        assert_eq!(plan.action.smp_affinity, "2");
        assert!(proposals[0].confidence > 0.0);
    }

    #[test]
    fn irq_provider_emits_no_proposal_when_structured_irq_evidence_is_missing() {
        let provider = IrqAffinityProvider;
        let observation = observation();
        let system_context = system_context();

        let proposals = provider.propose(&CandidateProviderInput {
            observation: &observation,
            daemon_policy: &policy(),
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            system_context: &system_context,
            controller_state: &ControllerRuntimeState::default(),
            profiles: &[],
        });

        assert!(proposals.is_empty());
    }

    #[test]
    fn irq_provider_emits_no_proposal_when_suggested_cpu_mask_is_unrepresentable() {
        let provider = IrqAffinityProvider;
        let mut observation = observation();
        observation.objective_signals.irq_overlap_count = Some(1);
        observation.objective_signals.irq_worst_overlap_ns = Some(4_000_000);
        observation.objective_signals.irq_hot_irq = Some(146);
        observation.objective_signals.irq_hot_cpu = Some(130);
        observation.active_config_snapshot = Some(ActiveConfigSnapshot {
            irq: ActiveIrqSnapshot {
                per_irq: BTreeMap::from([(146, "4".to_owned())]),
            },
            ..ActiveConfigSnapshot::default()
        });

        let mut system_context = system_context();
        system_context.inventory.irq_lines = vec![IrqLine {
            irq: "146".to_owned(),
            counts_by_cpu: Vec::new(),
            total: 60_002,
            kind: "PCI-MSI".to_owned(),
            name: "amdgpu".to_owned(),
            raw: "146: PCI-MSI amdgpu".to_owned(),
        }];

        let proposals = provider.propose(&CandidateProviderInput {
            observation: &observation,
            daemon_policy: &policy(),
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            system_context: &system_context,
            controller_state: &ControllerRuntimeState::default(),
            profiles: &[],
        });

        assert!(proposals.is_empty());
    }

    #[test]
    fn irq_provider_avoids_audio_compositor_and_reserved_cpus() {
        let provider = IrqAffinityProvider;
        let mut observation = observation_with_irq_pressure();
        observation.active_tasks = vec![
            task(10, TaskClass::AudioRealtime),
            task(11, TaskClass::Compositor),
            task(12, TaskClass::KernelThread),
        ];
        observation.active_config_snapshot = Some(ActiveConfigSnapshot {
            affinity: ActiveAffinitySnapshot {
                per_tid: BTreeMap::from([
                    (10, "1".to_owned()),
                    (11, "2".to_owned()),
                    (12, "3".to_owned()),
                ]),
            },
            irq: ActiveIrqSnapshot {
                per_irq: BTreeMap::from([(146, "4".to_owned())]),
            },
            ..ActiveConfigSnapshot::default()
        });

        let mut system_context = system_context_with_irq_line(vec![50, 1, 2, 3]);
        system_context.inventory.irq_default_smp_affinity = Some("f".to_owned());

        let proposals = provider.propose(&CandidateProviderInput {
            observation: &observation,
            daemon_policy: &policy(),
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            system_context: &system_context,
            controller_state: &ControllerRuntimeState::default(),
            profiles: &[],
        });

        assert_eq!(proposals.len(), 1);
        let CandidateAction::IrqAffinity { plan } = &proposals[0].candidate else {
            panic!("expected irq affinity candidate");
        };
        assert_eq!(plan.action.smp_affinity, "1");
        assert!(plan.evidence[0].value.contains("audio_realtime_cpus={1}"));
        assert!(plan.evidence[0].value.contains("compositor_cpus={2}"));
        assert!(plan.evidence[0].value.contains("reserved_cpus={3}"));
    }

    #[test]
    fn irq_provider_prefers_housekeeping_over_focused_workload_cpu() {
        let provider = IrqAffinityProvider;
        let mut observation = observation_with_irq_pressure();
        observation.active_tasks = vec![task(10, TaskClass::GameRenderThread)];
        observation.active_config_snapshot = Some(ActiveConfigSnapshot {
            affinity: ActiveAffinitySnapshot {
                per_tid: BTreeMap::from([(10, "1".to_owned())]),
            },
            irq: ActiveIrqSnapshot {
                per_irq: BTreeMap::from([(146, "1".to_owned())]),
            },
            ..ActiveConfigSnapshot::default()
        });

        let mut system_context = system_context_with_irq_line(vec![20, 1, 50, 10]);
        system_context.inventory.irq_default_smp_affinity = Some("f".to_owned());

        let proposals = provider.propose(&CandidateProviderInput {
            observation: &observation,
            daemon_policy: &policy(),
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            system_context: &system_context,
            controller_state: &ControllerRuntimeState::default(),
            profiles: &[],
        });

        assert_eq!(proposals.len(), 1);
        let CandidateAction::IrqAffinity { plan } = &proposals[0].candidate else {
            panic!("expected irq affinity candidate");
        };
        assert_eq!(plan.action.smp_affinity, "8");
        assert!(plan.evidence[0].value.contains("selection=housekeeping"));
        assert!(plan.evidence[0].value.contains("focused_workload_cpus={1}"));
    }

    #[test]
    fn irq_provider_rejects_when_only_candidate_cpu_is_protected() {
        let provider = IrqAffinityProvider;
        let mut observation = observation_with_irq_pressure();
        observation.objective_signals.irq_hot_cpu = Some(1);
        observation.active_tasks = vec![task(10, TaskClass::AudioRealtime)];
        observation.active_config_snapshot = Some(ActiveConfigSnapshot {
            affinity: ActiveAffinitySnapshot {
                per_tid: BTreeMap::from([(10, "0".to_owned())]),
            },
            irq: ActiveIrqSnapshot {
                per_irq: BTreeMap::from([(146, "2".to_owned())]),
            },
            ..ActiveConfigSnapshot::default()
        });

        let mut system_context = system_context_with_irq_line(vec![1, 2]);
        system_context.inventory.irq_default_smp_affinity = Some("1".to_owned());

        let proposals = provider.propose(&CandidateProviderInput {
            observation: &observation,
            daemon_policy: &policy(),
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            system_context: &system_context,
            controller_state: &ControllerRuntimeState::default(),
            profiles: &[],
        });

        assert!(proposals.is_empty());
    }

    fn observation_with_irq_pressure() -> AutotuneObservation {
        let mut observation = observation();
        observation.objective_signals.irq_overlap_count = Some(2);
        observation.objective_signals.irq_worst_overlap_ns = Some(4_000_000);
        observation.objective_signals.irq_hot_irq = Some(146);
        observation.objective_signals.irq_hot_cpu = Some(2);
        observation.active_config_snapshot = Some(ActiveConfigSnapshot {
            irq: ActiveIrqSnapshot {
                per_irq: BTreeMap::from([(146, "4".to_owned())]),
            },
            ..ActiveConfigSnapshot::default()
        });
        observation
    }

    fn system_context_with_irq_line(counts_by_cpu: Vec<u64>) -> SystemContextSnapshot {
        let mut system_context = system_context();
        system_context.inventory.irq_lines = vec![IrqLine {
            irq: "146".to_owned(),
            total: counts_by_cpu.iter().sum(),
            counts_by_cpu,
            kind: "PCI-MSI".to_owned(),
            name: "amdgpu".to_owned(),
            raw: "146: PCI-MSI amdgpu".to_owned(),
        }];
        system_context
    }

    fn task(tid: u32, class: TaskClass) -> ActiveTaskSnapshot {
        ActiveTaskSnapshot {
            tid,
            process_pid: tid,
            comm: format!("task-{tid}"),
            class,
            process_starttime_ticks: Some(1000 + tid as u64),
            task_starttime_ticks: Some(2000 + tid as u64),
            cgroup_path: None,
        }
    }

    fn observation() -> AutotuneObservation {
        let mut observation = AutotuneObservation {
            target_present: true,
            target_root_pid: Some(1234),
            primary_situation: SituationKind::IrqPressure,
            focus_kind: Some(FocusGroupKind::Game),
            focus_confidence: 0.95,
            capabilities: DaemonCapabilities {
                irq_affinity_available: true,
                ..DaemonCapabilities::default()
            },
            system_health: SystemHealthSnapshot {
                ok_for_apply: true,
                ..SystemHealthSnapshot::default()
            },
            ..AutotuneObservation::default()
        };
        observation.refresh_situation_classification();
        observation.primary_situation = SituationKind::IrqPressure;
        observation
    }

    fn system_context() -> SystemContextSnapshot {
        SystemContextSnapshot {
            capabilities: DaemonCapabilities::default(),
            health: SystemHealthSnapshot::default(),
            inventory: SystemInventory {
                cpu_policies: Vec::new(),
                drm_devices: Vec::new(),
                irq_default_smp_affinity: None,
                irq_lines: Vec::new(),
                power_source: Default::default(),
                sched_ext_available: false,
                vm_knobs: Default::default(),
                inventory_hash: "irq-test".to_owned(),
            },
            active_config: Default::default(),
            sampled_at_unix_nanos: 10,
        }
    }

    fn policy() -> crate::daemon::DaemonPolicy {
        let config = crate::autotune::runtime::daemon_config_for_runtime_mode(
            DaemonMode::Suggest,
            ActionSource::AutotuneRuntime,
            Some(1234),
            None,
        );
        build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: None,
        })
    }
}
