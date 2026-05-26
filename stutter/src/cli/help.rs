use clap::error::ErrorKind;

pub(crate) fn is_successful_clap_display_error(err: &anyhow::Error) -> bool {
    err.downcast_ref::<clap::Error>().is_some_and(|err| {
        matches!(
            err.kind(),
            ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
        )
    })
}

pub(crate) fn print_clap_display_error(
    err: anyhow::Error,
) -> Result<(), crate::error::StutterError> {
    let err = err
        .downcast::<clap::Error>()
        .map_err(crate::error::StutterError::Command)?;
    err.print()
        .map_err(|err| crate::error::StutterError::Command(err.into()))
}
