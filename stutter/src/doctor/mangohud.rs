use std::{collections::BTreeMap, fs, io::Read, path::Path};
use super::model::{DoctorCheck, DoctorStatus};

pub fn check_mangohud_log_path(path: &Path) -> DoctorCheck {
    let mut details = BTreeMap::new();
    details.insert("path".to_owned(), path.display().to_string());

    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(err) => {
            details.insert("error".to_owned(), err.to_string());
            return DoctorCheck {
                name: "mangohud_log".to_owned(),
                status: DoctorStatus::Warn,
                message: "MangoHud log is missing or unreadable".to_owned(),
                details,
            };
        }
    };

    let mut buf = String::new();
    if let Err(err) = file.by_ref().take(8192).read_to_string(&mut buf) {
        details.insert("error".to_owned(), err.to_string());
        return DoctorCheck {
            name: "mangohud_log".to_owned(),
            status: DoctorStatus::Warn,
            message: "MangoHud log could not be read".to_owned(),
            details,
        };
    }

    if buf.trim().is_empty() {
        return DoctorCheck {
            name: "mangohud_log".to_owned(),
            status: DoctorStatus::Warn,
            message: "MangoHud log is empty".to_owned(),
            details,
        };
    }

    let looks_csv = buf.lines().any(|line| {
        let parts = line.split(',').map(str::trim).collect::<Vec<_>>();
        parts.len() >= 2 && parts.iter().take(2).all(|part| !part.is_empty())
    });

    DoctorCheck {
        name: "mangohud_log".to_owned(),
        status: if looks_csv {
            DoctorStatus::Pass
        } else {
            DoctorStatus::Warn
        },
        message: if looks_csv {
            "MangoHud log looks like comma-separated telemetry".to_owned()
        } else {
            "MangoHud log does not look like CSV telemetry".to_owned()
        },
        details,
    }
}
