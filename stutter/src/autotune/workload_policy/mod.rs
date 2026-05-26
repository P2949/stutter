mod defaults;
mod lint;
#[path = "match.rs"]
mod r#match;
mod model;
mod parse;

pub use defaults::workload_policy_for_situation;
pub use lint::{lint_workload_policy, validate_workload_policy_lints};
pub use model::{
    DaemonWorkloadPolicyConfig, DaemonWorkloadPolicyConfigFile, LintSeverity, WorkloadPolicyLint,
    WorkloadPolicyMatrix, WorkloadPolicyRule, WorkloadPolicyRuleConfigFile,
};
pub use parse::{
    known_action_families, parse_objective_kind, parse_situation_kind,
    parse_workload_policy_rule_configs, validate_action_family_name, validate_workload_policy_rule,
};

#[cfg(test)]
mod tests;
