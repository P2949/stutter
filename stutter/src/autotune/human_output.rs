use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HumanControllerPhase {
    Disabled,
    Observing,
    Planning,
    Applying,
    Measuring,
    Keeping,
    Reverting,
    Cooldown,
    Faulted,
}

impl fmt::Display for HumanControllerPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Disabled => "Disabled",
            Self::Observing => "Observing",
            Self::Planning => "Planning",
            Self::Applying => "Applying",
            Self::Measuring => "Measuring",
            Self::Keeping => "Keeping",
            Self::Reverting => "Reverting",
            Self::Cooldown => "Cooldown",
            Self::Faulted => "Faulted",
        };
        f.write_str(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HumanAutotuneMode {
    Observe,
    Suggest,
    ApplyLowRisk,
    ApplyMediumRisk,
    ApplyHighRisk,
}

impl fmt::Display for HumanAutotuneMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Observe => "Observe",
            Self::Suggest => "Suggest",
            Self::ApplyLowRisk => "ApplyLowRisk",
            Self::ApplyMediumRisk => "ApplyMediumRisk",
            Self::ApplyHighRisk => "ApplyHighRisk",
        };
        f.write_str(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HumanSituationKind {
    Unknown,
    Idle,
    GameFocused,
    GameCpuSchedulerPressure,
    GameGpuBound,
    CompositorPressure,
    CpuPressure,
    IoPressure,
    IrqPressure,
    ThermalOrPowerLimit,
    CompileLoad,
}

impl fmt::Display for HumanSituationKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Unknown => "Unknown",
            Self::Idle => "Idle",
            Self::GameFocused => "GameFocused",
            Self::GameCpuSchedulerPressure => "GameCpuSchedulerPressure",
            Self::GameGpuBound => "GameGpuBound",
            Self::CompositorPressure => "CompositorPressure",
            Self::CpuPressure => "CpuPressure",
            Self::IoPressure => "IoPressure",
            Self::IrqPressure => "IrqPressure",
            Self::ThermalOrPowerLimit => "ThermalOrPowerLimit",
            Self::CompileLoad => "CompileLoad",
        };
        f.write_str(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HumanDecisionKind {
    Noop,
    Suggest,
    StartExperiment,
    KeepCurrent,
    Revert,
    EnterCooldown,
    Fault,
}

impl HumanDecisionKind {
    pub fn as_human_str(self) -> &'static str {
        match self {
            Self::Noop => "noop",
            Self::Suggest => "suggest",
            Self::StartExperiment => "start-experiment",
            Self::KeepCurrent => "keep-current",
            Self::Revert => "revert",
            Self::EnterCooldown => "enter-cooldown",
            Self::Fault => "fault",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HumanDecisionWindow {
    pub phase: HumanControllerPhase,
    pub mode: HumanAutotuneMode,
    pub target: String,
    pub score_total: u64,
    pub situation: HumanSituationKind,
    pub decision: HumanDecisionKind,
    pub reason: String,
    pub planner_summary: Option<String>,
}

impl HumanDecisionWindow {
    pub fn observe_noop(
        target: impl Into<String>,
        score_total: u64,
        situation: HumanSituationKind,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            phase: HumanControllerPhase::Observing,
            mode: HumanAutotuneMode::Observe,
            target: target.into(),
            score_total,
            situation,
            decision: HumanDecisionKind::Noop,
            reason: reason.into(),
            planner_summary: None,
        }
    }
}

pub fn render_human_decision_window(window: &HumanDecisionWindow) -> String {
    let mut line = format!(
        "autotune phase={} mode={} target={} score={} situation={} decision={} reason=\"{}\"",
        window.phase,
        window.mode,
        shell_safe_value(&window.target),
        window.score_total,
        window.situation,
        window.decision.as_human_str(),
        escape_reason(&window.reason)
    );

    if let Some(planner_summary) = window.planner_summary.as_ref() {
        line.push_str(&format!(" planner=\"{}\"", escape_reason(planner_summary)));
    }

    line
}

pub fn print_human_decision_window(window: &HumanDecisionWindow) {
    println!("{}", render_human_decision_window(window));
}

fn shell_safe_value(value: &str) -> String {
    if value.is_empty() {
        return "-".to_owned();
    }

    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | ':' | '/' | '@'))
    {
        return value.to_owned();
    }

    format!("\"{}\"", escape_reason(value))
}

fn escape_reason(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());

    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            other => escaped.push(other),
        }
    }

    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_requested_observe_noop_line() {
        let window = HumanDecisionWindow::observe_noop(
            "KingdomCome.exe",
            143,
            HumanSituationKind::GameCpuSchedulerPressure,
            "observe mode",
        );

        let line = render_human_decision_window(&window);

        assert_eq!(
            line,
            "autotune phase=Observing mode=Observe target=KingdomCome.exe score=143 situation=GameCpuSchedulerPressure decision=noop reason=\"observe mode\""
        );
    }

    #[test]
    fn quotes_target_when_it_contains_spaces() {
        let window = HumanDecisionWindow::observe_noop(
            "Kingdom Come.exe",
            143,
            HumanSituationKind::GameCpuSchedulerPressure,
            "observe mode",
        );

        let line = render_human_decision_window(&window);

        assert_eq!(
            line,
            "autotune phase=Observing mode=Observe target=\"Kingdom Come.exe\" score=143 situation=GameCpuSchedulerPressure decision=noop reason=\"observe mode\""
        );
    }

    #[test]
    fn escapes_reason_quotes_and_newlines() {
        let window = HumanDecisionWindow::observe_noop(
            "KingdomCome.exe",
            143,
            HumanSituationKind::GameCpuSchedulerPressure,
            "observe \"mode\"\nnext",
        );

        let line = render_human_decision_window(&window);

        assert_eq!(
            line,
            "autotune phase=Observing mode=Observe target=KingdomCome.exe score=143 situation=GameCpuSchedulerPressure decision=noop reason=\"observe \\\"mode\\\"\\nnext\""
        );
    }

    #[test]
    fn renders_planner_summary_when_present() {
        let mut window = HumanDecisionWindow::observe_noop(
            "Game.exe",
            143,
            HumanSituationKind::GameCpuSchedulerPressure,
            "no candidate selected",
        );
        window.planner_summary = Some(
            "total=3 eligible=0 grouped=capability_missing=2 denied=nice-candidate".to_owned(),
        );

        let line = render_human_decision_window(&window);

        assert!(line.contains(
            "planner=\"total=3 eligible=0 grouped=capability_missing=2 denied=nice-candidate\""
        ));
    }

    #[test]
    fn renders_non_noop_decisions_in_kebab_case() {
        let window = HumanDecisionWindow {
            phase: HumanControllerPhase::Cooldown,
            mode: HumanAutotuneMode::Suggest,
            target: "Game.exe".to_owned(),
            score_total: 200,
            situation: HumanSituationKind::CpuPressure,
            decision: HumanDecisionKind::EnterCooldown,
            reason: "cooldown blocks repeated action".to_owned(),
            planner_summary: None,
        };

        let line = render_human_decision_window(&window);

        assert_eq!(
            line,
            "autotune phase=Cooldown mode=Suggest target=Game.exe score=200 situation=CpuPressure decision=enter-cooldown reason=\"cooldown blocks repeated action\""
        );
    }
}
