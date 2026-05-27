use std::{fs, io, path::Path};
use crate::drm_tracepoints::DrmTracepointFormat;

pub(crate) fn format_rlimit_bytes(value: u64) -> String {
    if value == libc::RLIM_INFINITY {
        "unlimited".to_owned()
    } else {
        value.to_string()
    }
}

pub(crate) fn read_trimmed(path: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(value) => Ok(Some(value.trim().to_owned())),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.to_string()),
    }
}

pub(crate) fn format_tracepoint_ref(format: Option<&DrmTracepointFormat>) -> String {
    format
        .map(|format| format!("{}/{}", format.category, format.name))
        .unwrap_or_else(|| "-".to_owned())
}

pub(crate) fn format_tracepoint_names(formats: &[DrmTracepointFormat]) -> String {
    if formats.is_empty() {
        "-".to_owned()
    } else {
        formats
            .iter()
            .map(|format| format.name.as_str())
            .collect::<Vec<_>>()
            .join(",")
    }
}

pub(crate) fn available_unavailable(value: bool) -> String {
    if value {
        "available".to_owned()
    } else {
        "unavailable".to_owned()
    }
}

pub(crate) fn yes_no(value: bool) -> String {
    if value {
        "yes".to_owned()
    } else {
        "no".to_owned()
    }
}
