use std::ffi::OsString;

pub(super) fn requested_version_features(argv: &[OsString]) -> bool {
    let mut version = false;
    let mut features = false;
    for arg in argv.iter().skip(1).filter_map(|arg| arg.to_str()) {
        match arg {
            "--version" | "-V" => version = true,
            "--features" => features = true,
            _ => {}
        }
    }
    version && features
}
