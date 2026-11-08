use super::*;
use crate::autotune::objective::source::ContractError;

fn binding() -> SourceBinding {
    SourceBinding {
        uid: 1000,
        pid: 42,
        start_ticks: 7,
    }
}

#[test]
fn both_templates_are_valid_versioned_experimental_contracts_that_authorize_nothing() {
    for template in [audio_xrun(binding()), browser_response(binding())] {
        template.contract.validate().unwrap();
        assert_eq!(template.version, TEMPLATE_VERSION);
        assert!(template.version.starts_with("experimental"));
        assert!(!template.contract.authorizes_mutation());
        assert!(!template.contract.hard_guardrails.is_empty());
        assert_eq!(
            template.contract.primary.direction,
            Direction::LowerIsBetter
        );
    }
}

#[test]
fn audio_is_judged_on_deadline_misses_with_xruns_as_a_hard_guardrail_and_browser_on_response_tail()
{
    let audio = audio_xrun(binding()).contract;
    assert_eq!(audio.primary.class, MetricClass::AudioDeadline);
    assert_eq!(audio.primary.id, "deadline_miss_count");
    assert_eq!(audio.hard_guardrails[0].id, "xrun_count");
    let browser = browser_response(binding()).contract;
    assert_eq!(browser.primary.class, MetricClass::Interactivity);
    assert_eq!(browser.primary.aggregation, Aggregation::P95);
    assert_eq!(browser.hard_guardrails[0].id, "long_task_count");
}

#[test]
fn a_template_cannot_be_weakened_into_an_invalid_contract_unnoticed() {
    let mut contract = audio_xrun(binding()).contract;
    contract.hard_guardrails.clear();
    assert_eq!(contract.validate(), Err(ContractError::NoHardGuardrail));
    let mut contract = browser_response(binding()).contract;
    contract.primary.equivalent_within_percent = contract.primary.meaningful_effect_percent;
    assert!(matches!(
        contract.validate(),
        Err(ContractError::BadSpec(..))
    ));
}

#[test]
fn a_template_contract_round_trips_as_the_json_kpi_serve_reads() {
    let json = serde_json::to_string(&audio_xrun(binding()).contract).unwrap();
    let back: KpiContract = serde_json::from_str(&json).unwrap();
    back.validate().unwrap();
    assert_eq!(back, audio_xrun(binding()).contract);
}
