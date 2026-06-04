use crate::profiles::explain::{ActionDecisionDto, ProfileExplainReport, ProfileTaskExplain};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProfileExplainRenderOptions {
    pub top: usize,
    pub highlight_comm: Vec<String>,
    pub include_tasks: bool,
}

impl Default for ProfileExplainRenderOptions {
    fn default() -> Self {
        Self {
            top: 10,
            highlight_comm: Vec::new(),
            include_tasks: true,
        }
    }
}

pub(crate) fn render_profile_explain_text(
    report: &ProfileExplainReport,
    options: &ProfileExplainRenderOptions,
) -> String {
    let mut out = String::new();

    out.push_str(&format!("Profile plan: {}\n\n", report.profile));
    out.push_str("Snapshot:\n");
    if let Some(tree_pid) = report.tree_pid {
        out.push_str(&format!("  tree pid:              {tree_pid}\n"));
    }
    out.push_str(&format!(
        "  snapshot tasks:        {}\n",
        report.snapshot_tasks
    ));
    out.push_str(&format!(
        "  matched tasks:         {}\n",
        report.matched_tasks
    ));
    out.push_str(&format!(
        "  unmatched tasks:       {}\n",
        report.unmatched_tasks
    ));
    out.push_str(&format!(
        "  pending unique tasks:  {}\n",
        report.pending_unique_tasks
    ));
    out.push_str(&format!(
        "  pending affinity:      {}\n",
        report.pending_affinity
    ));
    out.push_str(&format!(
        "  pending nice:          {}\n",
        report.pending_nice
    ));
    out.push_str(&format!(
        "  pending ionice:        {}\n",
        report.pending_ionice
    ));
    out.push_str(
        "\nRule matching is first-match-wins: each task is assigned to the first matching rule.\n",
    );

    for rule in &report.rules {
        out.push('\n');
        out.push_str(&format!("Rule {}\n", rule.rule_index));
        out.push_str("  action:\n");
        if let Some(affinity) = &rule.actions.affinity {
            out.push_str(&format!("    affinity: {affinity}\n"));
        }
        if let Some(nice) = rule.actions.nice {
            out.push_str(&format!("    nice: {nice}\n"));
        }
        if let Some(ionice) = &rule.actions.ionice {
            out.push_str(&format!("    ionice: {ionice}\n"));
        }

        out.push_str("  match:\n");
        if !rule.match_class.is_empty() {
            out.push_str(&format!("    classes: {}\n", rule.match_class.join(", ")));
        }
        if !rule.match_comm.is_empty() {
            out.push_str(&format!(
                "    comm patterns: {}\n",
                rule.match_comm.join(", ")
            ));
        }
        if rule.match_class.is_empty() && rule.match_comm.is_empty() {
            out.push_str("    catch-all\n");
        }

        out.push('\n');
        out.push_str(&format!(
            "  matched tasks:         {}\n",
            rule.matched_tasks
        ));
        out.push_str(&format!(
            "  pending unique tasks:  {}\n",
            rule.pending_unique_tasks
        ));
        out.push_str(&format!(
            "  pending affinity:      {}\n",
            rule.pending_affinity
        ));
        out.push_str(&format!(
            "  already satisfied:     {}\n",
            rule.already_satisfied_tasks
        ));
        if rule.skipped_tasks > 0 {
            out.push_str(&format!(
                "  skipped tasks:         {}\n",
                rule.skipped_tasks
            ));
        }

        out.push('\n');
        out.push_str("  match source:\n");
        out.push_str(&format!(
            "    task.comm:           {}\n",
            rule.match_basis.task_comm
        ));
        out.push_str(&format!(
            "    process_comm:        {}\n",
            rule.match_basis.process_comm
        ));
        out.push_str(&format!(
            "    both comm fields:    {}\n",
            rule.match_basis.both_comm_fields
        ));
        out.push_str(&format!(
            "    class only:          {}\n",
            rule.match_basis.class_only
        ));
        out.push_str(&format!(
            "    catch-all:           {}\n",
            rule.match_basis.catch_all
        ));

        render_map_section(&mut out, "  classes:", &rule.classes, options.top);
        render_map_section(
            &mut out,
            "  top thread comms:",
            &rule.top_thread_comms,
            options.top,
        );
        render_map_section(
            &mut out,
            "  top process comms:",
            &rule.top_process_comms,
            options.top,
        );

        if !rule.broad_process_comm_captured_thread_comms.is_empty() {
            render_map_section(
                &mut out,
                "  broad process_comm captures:",
                &rule.broad_process_comm_captured_thread_comms,
                options.top,
            );
            let captures: usize = rule.broad_process_comm_captured_thread_comms.values().sum();
            out.push('\n');
            out.push_str("  Note:\n");
            out.push_str(&format!(
                "    Rule {} captured {} task(s) through process_comm match_comm = [{}] while their own thread comm differed.\n",
                rule.rule_index,
                captures,
                rule.match_comm.join(", ")
            ));
        }
    }

    render_highlighted_tasks(report, options, &mut out);

    if report.unmatched.tasks > 0 {
        out.push('\n');
        out.push_str("Unmatched:\n");
        out.push_str(&format!("  tasks: {}\n", report.unmatched.tasks));
        render_map_section(
            &mut out,
            "  classes:",
            &report.unmatched.classes,
            options.top,
        );
        render_map_section(
            &mut out,
            "  top thread comms:",
            &report.unmatched.top_thread_comms,
            options.top,
        );
    }

    if !report.warnings.is_empty() {
        out.push('\n');
        out.push_str("Warnings:\n");
        for warning in &report.warnings {
            out.push_str(&format!("  {warning}\n"));
        }
    }

    out
}

fn render_map_section(
    out: &mut String,
    title: &str,
    map: &std::collections::BTreeMap<String, usize>,
    top: usize,
) {
    if map.is_empty() {
        return;
    }

    out.push('\n');
    out.push_str(title);
    out.push('\n');
    for (key, count) in top_entries(map, top) {
        out.push_str(&format!("    {key}: {count}\n"));
    }
}

fn top_entries(
    map: &std::collections::BTreeMap<String, usize>,
    top: usize,
) -> Vec<(&String, &usize)> {
    let mut entries = map.iter().collect::<Vec<_>>();
    entries.sort_by(|(left_key, left_count), (right_key, right_count)| {
        right_count
            .cmp(left_count)
            .then_with(|| left_key.cmp(right_key))
    });
    entries.truncate(top);
    entries
}

fn render_highlighted_tasks(
    report: &ProfileExplainReport,
    options: &ProfileExplainRenderOptions,
    out: &mut String,
) {
    if !options.include_tasks || options.highlight_comm.is_empty() {
        return;
    }

    let highlights = report
        .rules
        .iter()
        .flat_map(|rule| &rule.tasks)
        .filter(|task| task_matches_highlight(task, &options.highlight_comm))
        .collect::<Vec<_>>();
    if highlights.is_empty() {
        return;
    }

    out.push('\n');
    out.push_str("Highlighted tasks:\n");
    out.push_str("  tid     comm              process_comm       class              rule  match source   affinity\n");
    for task in highlights {
        out.push_str(&format!(
            "  {:<7} {:<17} {:<18} {:<18} {:<5} {:<14} {}\n",
            task.tid,
            truncate(&task.comm, 17),
            truncate(&task.process_comm, 18),
            truncate(&task.class, 18),
            task.matched_rule_index,
            task_match_source(task),
            action_summary(task.affinity.as_ref())
        ));
    }
}

fn task_matches_highlight(task: &ProfileTaskExplain, patterns: &[String]) -> bool {
    let comm = task.comm.to_ascii_lowercase();
    let process_comm = task.process_comm.to_ascii_lowercase();
    patterns.iter().any(|pattern| {
        let pattern = pattern.to_ascii_lowercase();
        comm.contains(&pattern) || process_comm.contains(&pattern)
    })
}

fn task_match_source(task: &ProfileTaskExplain) -> &'static str {
    let matched_task_comm = task
        .match_evidence
        .comm_hits
        .iter()
        .any(|hit| matches!(hit.field, crate::profiles::explain::CommFieldDto::TaskComm));
    let matched_process_comm = task.match_evidence.comm_hits.iter().any(|hit| {
        matches!(
            hit.field,
            crate::profiles::explain::CommFieldDto::ProcessComm
        )
    });

    match (
        matched_task_comm,
        matched_process_comm,
        task.match_evidence.matched_class,
    ) {
        (true, true, _) => "both",
        (true, false, _) => "task_comm",
        (false, true, _) => "process_comm",
        (false, false, true) => "class",
        (false, false, false) => "catch_all",
    }
}

fn action_summary(decision: Option<&ActionDecisionDto>) -> String {
    let Some(decision) = decision else {
        return "none".to_owned();
    };

    match (&decision.current, &decision.desired) {
        (Some(current), Some(desired)) if current != desired => {
            format!("{current} -> {desired} {:?}", decision.status)
        }
        (Some(current), Some(_)) => format!("{current} {:?}", decision.status),
        _ => format!("{:?}", decision.status),
    }
}

fn truncate(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_owned();
    }
    value
        .chars()
        .take(width.saturating_sub(1))
        .collect::<String>()
        + "~"
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use stutter_core::ids::{Pid, Tid};

    use super::*;
    use crate::profiles::explain::{
        ActionDecisionDto, ActionStatus, CommFieldDto, CommPatternHitDto, MatchBasisCounts,
        ProfileRuleActionExplain, ProfileRuleExplain, ProfileTaskExplain, ProfileUnmatchedExplain,
        RuleMatchEvidenceDto,
    };

    fn report_for_render() -> ProfileExplainReport {
        ProfileExplainReport {
            schema_version: 1,
            profile: "game".to_owned(),
            tree_pid: Some(Pid::new(42)),
            snapshot_tasks: 3,
            matched_tasks: 2,
            unmatched_tasks: 1,
            pending_unique_tasks: 1,
            pending_affinity: 1,
            pending_nice: 0,
            pending_ionice: 0,
            rules: vec![ProfileRuleExplain {
                rule_index: 0,
                actions: ProfileRuleActionExplain {
                    affinity: Some("1-5,7-11".to_owned()),
                    nice: None,
                    ionice: None,
                },
                match_class: Vec::new(),
                match_comm: vec!["Main".to_owned()],
                matched_tasks: 2,
                pending_unique_tasks: 1,
                pending_affinity: 1,
                pending_nice: 0,
                pending_ionice: 0,
                already_satisfied_tasks: 1,
                skipped_tasks: 0,
                match_basis: MatchBasisCounts {
                    process_comm: 2,
                    ..MatchBasisCounts::default()
                },
                classes: BTreeMap::from([
                    ("Helper".to_owned(), 2),
                    ("GameRenderThread".to_owned(), 1),
                ]),
                top_thread_comms: BTreeMap::from([
                    ("RenderThread".to_owned(), 1),
                    ("ClothingRaycast".to_owned(), 1),
                    ("AudioThread".to_owned(), 2),
                ]),
                top_process_comms: BTreeMap::from([("Main".to_owned(), 2)]),
                broad_process_comm_captured_thread_comms: BTreeMap::from([
                    ("RenderThread".to_owned(), 1),
                    ("ClothingRaycast".to_owned(), 1),
                ]),
                tasks: vec![ProfileTaskExplain {
                    tid: Tid::new(100),
                    process_pid: Pid::new(42),
                    comm: "RenderThread".to_owned(),
                    process_comm: "Main".to_owned(),
                    class: "GameRenderThread".to_owned(),
                    matched_rule_index: 0,
                    match_evidence: RuleMatchEvidenceDto {
                        matched_class: false,
                        comm_hits: vec![CommPatternHitDto {
                            field: CommFieldDto::ProcessComm,
                            pattern: "Main".to_owned(),
                            value: "Main".to_owned(),
                        }],
                    },
                    affinity: Some(ActionDecisionDto {
                        status: ActionStatus::Pending,
                        current: Some("0-11".to_owned()),
                        desired: Some("1-5,7-11".to_owned()),
                        reason: None,
                    }),
                    nice: None,
                    ionice: None,
                    pending: true,
                }],
            }],
            unmatched: ProfileUnmatchedExplain {
                tasks: 1,
                classes: BTreeMap::from([("Compositor".to_owned(), 1)]),
                top_thread_comms: BTreeMap::from([("kwin_wayland".to_owned(), 1)]),
                top_process_comms: BTreeMap::new(),
            },
            warnings: Vec::new(),
        }
    }

    #[test]
    fn render_explain_includes_rule_summary() {
        let text = render_profile_explain_text(
            &report_for_render(),
            &ProfileExplainRenderOptions::default(),
        );

        assert!(text.contains("Profile plan: game"));
        assert!(text.contains("Rule 0"));
        assert!(text.contains("pending affinity:      1"));
    }

    #[test]
    fn render_explain_includes_profile_audit_clues() {
        let text = render_profile_explain_text(
            &report_for_render(),
            &ProfileExplainRenderOptions::default(),
        );

        assert!(text.contains("first-match-wins"));
        assert!(text.contains("process_comm"));
        assert!(text.contains("RenderThread"));
        assert!(text.contains("ClothingRaycast"));
        assert!(text.contains("pending affinity"));
    }

    #[test]
    fn render_explain_includes_broad_process_comm_note() {
        let text = render_profile_explain_text(
            &report_for_render(),
            &ProfileExplainRenderOptions::default(),
        );

        assert!(text.contains("broad process_comm captures"));
        assert!(text.contains("captured 2 task(s) through process_comm match_comm = [Main]"));
    }

    #[test]
    fn render_explain_includes_highlighted_tasks() {
        let text = render_profile_explain_text(
            &report_for_render(),
            &ProfileExplainRenderOptions {
                highlight_comm: vec!["Render".to_owned()],
                ..ProfileExplainRenderOptions::default()
            },
        );

        assert!(text.contains("Highlighted tasks"));
        assert!(text.contains("RenderThread"));
        assert!(text.contains("process_comm"));
    }

    #[test]
    fn render_explain_truncates_top_comms_in_text_only() {
        let text = render_profile_explain_text(
            &report_for_render(),
            &ProfileExplainRenderOptions {
                top: 1,
                ..ProfileExplainRenderOptions::default()
            },
        );

        assert!(text.contains("AudioThread: 2"));
        assert!(!text.contains("GameRenderThread: 1\n"));
    }
}
