use super::*;

#[derive(Args, Debug, Clone)]
pub(super) struct ReleaseArgs {
    #[command(subcommand)]
    pub(super) command: ReleaseCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub(super) enum ReleaseCommand {
    Check(ReleaseCheckArgs),
}

#[derive(Args, Debug, Clone)]
pub(super) struct ReleaseCheckArgs {
    #[arg(long = "channel", default_value = "experimental")]
    pub(super) channel: String,

    #[arg(long = "apply-actions-enabled")]
    pub(super) apply_actions_enabled: bool,

    #[arg(long = "soak-tests")]
    pub(super) soak_tests: bool,

    #[arg(long = "stronger-tests")]
    pub(super) stronger_tests: bool,

    #[arg(long = "real-machine-validation")]
    pub(super) real_machine_validation: bool,

    #[arg(long = "real-validation-matrix")]
    pub(super) real_validation_matrix: bool,

    #[arg(long = "false-negative-catalogue")]
    pub(super) false_negative_catalogue: bool,

    #[arg(long = "multi-machine-validation")]
    pub(super) multi_machine_validation: bool,

    #[arg(long = "local-install-smoke-tests")]
    pub(super) local_install_smoke_tests: bool,

    #[arg(long = "service-doctor-smoke-tests")]
    pub(super) service_doctor_smoke_tests: bool,

    #[arg(long = "emergency-restore-smoke-tests")]
    pub(super) emergency_restore_smoke_tests: bool,

    #[arg(long = "unprivileged-report-smoke-tests")]
    pub(super) unprivileged_report_smoke_tests: bool,

    #[arg(long = "packaged-artifact-layout-tests")]
    pub(super) packaged_artifact_layout_tests: bool,

    #[arg(long = "service-start-stop-smoke-tests")]
    pub(super) service_start_stop_smoke_tests: bool,

    #[arg(long = "rollback-drill")]
    pub(super) rollback_drill: bool,

    #[arg(
        long = "production-distro-packaging",
        help = "Mark production distro packaging as ready; defaults false because current ebuild/overlay packaging is skeleton-only"
    )]
    pub(super) production_distro_packaging: bool,

    #[arg(
        long = "reproducible-packaged-ebpf-object",
        help = "Mark packaged eBPF object build/artifact flow as reproducible"
    )]
    pub(super) reproducible_packaged_ebpf_object: bool,

    #[arg(
        long = "packaging-install-tests",
        help = "Mark distro packaging install/layout tests as passing"
    )]
    pub(super) packaging_install_tests: bool,

    #[arg(
        long = "packaging-service-smoke-tests",
        help = "Mark packaged service start/stop smoke tests as passing"
    )]
    pub(super) packaging_service_smoke_tests: bool,

    #[arg(
        long = "versioned-release-tarball",
        help = "Mark versioned release tarballs/artifacts as available for packagers"
    )]
    pub(super) versioned_release_tarball: bool,

    #[arg(long)]
    pub(super) json: bool,

    #[arg(long)]
    pub(super) enforce: bool,
}
