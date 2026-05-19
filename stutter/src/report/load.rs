use super::*;
use crate::artifacts::ArtifactSelection;

pub(crate) fn load_report_input(path: &Path) -> Result<ReportInputModel, ReportError> {
    let validation =
        session_io::validate_run_dir_shallow(path).map_err(|source| ReportError::Load {
            path: path.to_path_buf(),
            source,
        })?;
    log_run_validation(path, &validation);

    let artifacts =
        session_io::load_run_artifacts(path, ArtifactSelection::report()).map_err(|source| {
            ReportError::Load {
                path: path.to_path_buf(),
                source,
            }
        })?;
    Ok(ReportInputModel::from_artifacts(artifacts))
}

pub(crate) fn load_report_session(path: &Path) -> anyhow::Result<SessionFile> {
    let validation = session_io::validate_run_dir_shallow(path)?;
    log_run_validation(path, &validation);

    session_io::load_session(path)
}

fn log_run_validation(path: &Path, validation: &session_io::RunValidationReport) {
    if !validation.is_ok() {
        for err in &validation.errors {
            log::error!("run_dir_validation_error path={} err={err}", path.display());
        }
    }
    for warning in &validation.warnings {
        log::warn!(
            "run_dir_validation_warning path={} warn={warning}",
            path.display()
        );
    }
}
