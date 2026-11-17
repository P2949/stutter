//! Architecture guard: no experiment applies a candidate before the autonomous apply gate has decided.
//! `apply_candidate_for_runtime_with_executor` is the one function that applies a candidate for a live
//! experiment. It is called from exactly one place in production, and the gate
//! (`validate_autonomous_support`, which calls `autonomous_apply_gate`) must run first in that same
//! function, so no path can reach the apply without passing the gate.

use std::fs;

use super::{
    crate_src_root, relative_to_crate_root,
    scanners::{production_code_lines_outside_cfg_test_modules, rust_files_under},
};

const APPLY: &str = "apply_candidate_for_runtime_with_executor";
const GATE: &str = "validate_autonomous_support(";

#[test]
fn the_only_production_call_of_the_apply_comes_after_the_autonomous_gate_in_its_own_function() {
    let root = crate_src_root();
    let mut callers = Vec::new();
    for path in rust_files_under(&root) {
        let relative = relative_to_crate_root(&path);
        // Test files are not production call sites.
        if relative.contains("/tests/") || relative.ends_with("_tests.rs") {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        for (line, content) in production_code_lines_outside_cfg_test_modules(&text) {
            // A call (`.apply_candidate...(`), not the definition (`fn apply_candidate...`).
            if content.contains(&format!(".{APPLY}("))
                || content.contains(&format!("self.{APPLY}("))
            {
                callers.push((relative.clone(), line));
            }
        }
    }
    assert_eq!(
        callers.len(),
        1,
        "the apply may be reached from one place only, found {callers:?}"
    );
    let (file, apply_line) = &callers[0];
    assert!(
        file.ends_with("autotune/live_experiment/lifecycle.rs"),
        "{file}"
    );

    // In that function the gate is called, and before the apply.
    let text = fs::read_to_string(root.join(file.trim_start_matches("src/"))).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    // Reported line numbers are 1-based.
    let apply_index = apply_line - 1;
    let function_start = (0..apply_index)
        .rev()
        .find(|index| lines[*index].contains("fn start_candidate_experiment_with_executor"))
        .expect("the apply is called from the experiment start function");
    let gate_line = (function_start..apply_index)
        .find(|index| lines[*index].contains(GATE))
        .expect("the autonomous gate must be called before the apply in the same function");
    assert!(gate_line < apply_index);
    // The gate's own implementation calls the product-maturity gate.
    let manager = fs::read_to_string(root.join("autotune/live_experiment/manager.rs")).unwrap();
    assert!(
        manager.contains("autonomous_apply_gate(&gate)"),
        "the support validation must call autonomous_apply_gate"
    );
}
