use crate::{
    commands::input::ServiceCommandInput,
    service::{
        ServiceAction, ServiceDoctorReport, ServicePlan, build_service_plan, diagnose_service_plan,
        execute_service_plan,
    },
};

pub fn run_service_command(input: ServiceCommandInput) -> anyhow::Result<()> {
    let plan = build_service_plan(input.request);

    match plan.action {
        ServiceAction::Doctor => {
            let report = diagnose_service_plan(plan);
            if input.json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", render_service_doctor_text(&report));
            }
        }
        ServiceAction::Install | ServiceAction::Uninstall => {
            execute_service_plan(&plan)?;
            if input.json {
                println!("{}", serde_json::to_string_pretty(&plan)?);
            } else {
                print!("{}", render_service_plan_text(&plan));
            }
        }
    }

    Ok(())
}

fn render_service_plan_text(plan: &ServicePlan) -> String {
    let mut text = String::new();

    text.push_str("Service plan\n");
    text.push_str("============\n");
    text.push_str(&format!("action: {:?}\n", plan.action));
    text.push_str(&format!("manager: {}\n", plan.manager));
    text.push_str(&format!("mode: {}\n", plan.mode));
    text.push_str(&format!("dry_run: {}\n", plan.dry_run));
    text.push_str(&format!("unit_name: {}\n", plan.unit_name));
    text.push_str(&format!("unit_source: {}\n", plan.unit_source.display()));
    text.push_str(&format!("unit_target: {}\n", plan.unit_target.display()));
    text.push_str(&format!("config_dir: {}\n", plan.config_dir.display()));
    text.push_str(&format!("state_dir: {}\n", plan.state_dir.display()));
    text.push_str(&format!("log_dir: {}\n", plan.log_dir.display()));
    text.push_str(&format!("binary_path: {}\n", plan.binary_path.display()));
    text.push_str("\nSteps\n");
    for step in &plan.steps {
        text.push_str(&format!(
            "- {} {} ({})\n",
            step.action,
            step.path.display(),
            step.description
        ));
    }
    if !plan.warnings.is_empty() {
        text.push_str("\nWarnings\n");
        for warning in &plan.warnings {
            text.push_str(&format!("- {warning}\n"));
        }
    }
    if !plan.post_install_hints.is_empty() {
        text.push_str("\nNext commands\n");
        for hint in &plan.post_install_hints {
            text.push_str(&format!("- {hint}\n"));
        }
    }

    text
}

fn render_service_doctor_text(report: &ServiceDoctorReport) -> String {
    let mut text = render_service_plan_text(&report.plan);

    text.push_str("\nDoctor\n");
    text.push_str("------\n");
    text.push_str(&format!("ok: {}\n", report.ok));
    for check in &report.checks {
        text.push_str(&format!(
            "- {}: {} ({})\n",
            check.name,
            if check.ok { "ok" } else { "missing" },
            check.detail
        ));
    }

    text
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::service::{ServiceCommandRequest, ServiceManager, ServiceMode};

    fn plan() -> ServicePlan {
        build_service_plan(ServiceCommandRequest {
            action: ServiceAction::Install,
            manager: ServiceManager::SystemdSystem,
            mode: ServiceMode::SystemObserve,
            dry_run: true,
            unit_dir: Some(PathBuf::from("/tmp/stutter-units")),
            config_dir: PathBuf::from("/tmp/stutter-etc"),
            state_dir: PathBuf::from("/tmp/stutter-state"),
            log_dir: PathBuf::from("/tmp/stutter-log"),
            binary_path: PathBuf::from("/usr/bin/stutter"),
        })
    }

    #[test]
    fn service_plan_text_includes_dry_run_steps_and_hints() {
        let text = render_service_plan_text(&plan());

        assert!(text.contains("Service plan"));
        assert!(text.contains("dry_run: true"));
        assert!(text.contains("copy_unit"));
        assert!(text.contains("systemctl daemon-reload"));
    }

    #[test]
    fn service_doctor_text_includes_check_results() {
        let report = diagnose_service_plan(plan());
        let text = render_service_doctor_text(&report);

        assert!(text.contains("Doctor"));
        assert!(text.contains("packaged_unit"));
        assert!(text.contains("binary_path"));
    }
}
