use super::helpers::parse_report_command;
use crate::{commands::input::AppCommand, release::ReleaseChannel};

#[test]
fn parses_release_check_command() {
    let command = parse_report_command([
        "stutter",
        "release",
        "check",
        "--channel",
        "low-risk-stable",
        "--soak-tests",
        "--json",
    ])
    .unwrap();

    let AppCommand::ReleaseCheck(input) = command else {
        panic!("expected release check command");
    };

    assert_eq!(input.channel, ReleaseChannel::LowRiskStable);
    assert!(input.inputs.soak_tests);
    assert!(input.json);
    assert!(!input.enforce);

    assert!(!input.inputs.production_distro_packaging);
    assert!(!input.inputs.reproducible_packaged_ebpf_object);
    assert!(!input.inputs.packaging_install_tests);
    assert!(!input.inputs.packaging_service_smoke_tests);
    assert!(!input.inputs.versioned_release_tarball);
}

#[test]
fn parses_release_check_full_flags() {
    let command = parse_report_command([
        "stutter",
        "release",
        "check",
        "--channel",
        "experimental",
        "--apply-actions-enabled",
        "--soak-tests",
        "--stronger-tests",
        "--real-machine-validation",
        "--real-validation-matrix",
        "--false-negative-catalogue",
        "--multi-machine-validation",
        "--local-install-smoke-tests",
        "--service-doctor-smoke-tests",
        "--emergency-restore-smoke-tests",
        "--unprivileged-report-smoke-tests",
        "--packaged-artifact-layout-tests",
        "--service-start-stop-smoke-tests",
        "--rollback-drill",
        "--production-distro-packaging",
        "--reproducible-packaged-ebpf-object",
        "--packaging-install-tests",
        "--packaging-service-smoke-tests",
        "--versioned-release-tarball",
        "--json",
        "--enforce",
    ])
    .unwrap();

    let AppCommand::ReleaseCheck(input) = command else {
        panic!("expected release check command");
    };

    assert_eq!(input.channel, ReleaseChannel::Experimental);
    assert!(input.inputs.apply_actions_enabled);
    assert!(input.inputs.soak_tests);
    assert!(input.inputs.stronger_tests);
    assert!(input.inputs.real_machine_validation);
    assert!(input.inputs.real_validation_matrix);
    assert!(input.inputs.false_negative_catalogue);
    assert!(input.inputs.multi_machine_validation);
    assert!(input.inputs.local_install_smoke_tests);
    assert!(input.inputs.service_doctor_smoke_tests);
    assert!(input.inputs.emergency_restore_smoke_tests);
    assert!(input.inputs.unprivileged_report_smoke_tests);
    assert!(input.inputs.packaged_artifact_layout_tests);
    assert!(input.inputs.service_start_stop_smoke_tests);
    assert!(input.inputs.rollback_drill);
    assert!(input.inputs.production_distro_packaging);
    assert!(input.inputs.reproducible_packaged_ebpf_object);
    assert!(input.inputs.packaging_install_tests);
    assert!(input.inputs.packaging_service_smoke_tests);
    assert!(input.inputs.versioned_release_tarball);
    assert!(input.json);
    assert!(input.enforce);
}
