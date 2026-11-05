//! Root execution of a closed set of actions on the pinned bounded Stutter service.
//! Unit names, binary destinations, commands and signals are derived here, never from wire text.
use std::{
    fs,
    io::Write,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::{Value, json};
use stutter_core::hash::sha256_hex;

use crate::{
    authorization::{Denial, is_safe_identifier},
    model::{
        Authorization, BoundedServiceAction, ControllerKind, ErrorCode, ServiceComponent,
        ServiceSignal,
    },
    process::{service_bin_dir, stutter_source_path, verify_pinned, xtask_source_path},
};

mod fixture;
mod ownership;

const ROOT_PATH: &str = "/run/current-system/sw/bin:/usr/sbin:/usr/bin:/sbin:/bin";
fn failure(message: impl Into<String>) -> Denial {
    (ErrorCode::ExecutionFailed, message.into())
}

pub fn account_name(passwd: &str, uid: u32, gid: u32) -> Result<String, Denial> {
    let matches: Vec<_> = passwd
        .lines()
        .filter_map(|line| {
            let fields: Vec<_> = line.split(':').collect();
            (fields.len() >= 7
                && fields[2].parse::<u32>().ok() == Some(uid)
                && fields[3].parse::<u32>().ok() == Some(gid))
            .then(|| fields[0])
        })
        .collect();
    if uid == 0 || matches.len() != 1 || !is_safe_identifier(matches[0]) {
        return Err(failure(
            "campaign owner must resolve to one non-root local desktop account",
        ));
    }
    Ok(matches[0].to_owned())
}

pub fn unit_name(user: &str, component: ServiceComponent) -> Result<String, Denial> {
    if !is_safe_identifier(user) {
        return Err(failure("unsafe desktop account name"));
    }
    Ok(format!(
        "{}@{user}.service",
        match component {
            ServiceComponent::Controller => "stutter-autotune-bounded",
            ServiceComponent::Worker => "stutter-privileged-worker",
        }
    ))
}

fn systemctl(args: &[&str]) -> Result<String, Denial> {
    let output = Command::new("systemctl")
        .args(args)
        .env("PATH", ROOT_PATH)
        .output()
        .map_err(|error| failure(error.to_string()))?;
    if !output.status.success() {
        return Err(failure(format!(
            "fixed service systemctl action failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    String::from_utf8(output.stdout).map_err(|error| failure(error.to_string()))
}

fn protected_directory(path: &Path) -> Result<(), Denial> {
    for ancestor in path.ancestors() {
        let metadata = fs::metadata(ancestor).map_err(|error| failure(error.to_string()))?;
        if metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
            return Err(failure(
                "service binary directory must be root-owned and protected",
            ));
        }
    }
    Ok(())
}

fn pin_binary(auth: &Authorization) -> Result<PathBuf, Denial> {
    // Read and hash the exact bytes once; a writable checkout cannot race verification and copy.
    let stutter = read_authorized(&stutter_source_path(auth), &auth.stutter_sha256, "stutter")?;
    let xtask = read_authorized(&xtask_source_path(auth), &auth.xtask_sha256, "xtask")?;
    let dir = service_bin_dir(auth);
    let root = auth
        .service_bin_root
        .as_deref()
        .unwrap_or(Path::new("/var/lib/stutter"));
    // Only create children of protected directories, and make these code-only directories
    // traversable by the desktop controller even when the runner has a private umask.
    for path in [
        root.to_owned(),
        dir.parent()
            .ok_or_else(|| failure("missing service binary parent"))?
            .to_owned(),
        dir.clone(),
    ] {
        if !path.exists() {
            protected_directory(
                path.parent()
                    .ok_or_else(|| failure("missing protected parent"))?,
            )?;
            fs::create_dir(&path).map_err(|error| failure(error.to_string()))?;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
                .map_err(|error| failure(error.to_string()))?;
        }
        protected_directory(&path)?;
    }
    write_protected(&dir, "xtask", &xtask)?;
    write_protected(&dir, "stutter", &stutter)
}

fn read_authorized(path: &Path, expected: &str, what: &str) -> Result<Vec<u8>, Denial> {
    let bytes = fs::read(path).map_err(|error| failure(error.to_string()))?;
    if sha256_hex(&bytes) != expected {
        return Err(failure(format!(
            "service source {what} differs from root authorization"
        )));
    }
    Ok(bytes)
}

/// Writes the already-hashed bytes to a root-owned read-only file, atomically and durably.
fn write_protected(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, Denial> {
    let binary = dir.join(name);
    let temporary = dir.join(format!("{name}.tmp-{}", std::process::id()));
    let _ = fs::remove_file(&temporary);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| failure(error.to_string()))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| failure(error.to_string()))?;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o555))
        .map_err(|error| failure(error.to_string()))?;
    fs::rename(&temporary, &binary).map_err(|error| failure(error.to_string()))?;
    fs::File::open(dir)
        .and_then(|file| file.sync_all())
        .map_err(|error| failure(error.to_string()))?;
    Ok(binary)
}

/// Called only after peer/grant checks. It also checks the controller kind defensively.
pub fn execute(auth: &Authorization, action: BoundedServiceAction) -> Result<Value, Denial> {
    if auth.controller != ControllerKind::ServiceGraduation {
        return Err((
            ErrorCode::ActionNotPermitted,
            "not a service campaign".to_owned(),
        ));
    }
    let passwd = fs::read_to_string("/etc/passwd").map_err(|error| failure(error.to_string()))?;
    let user = account_name(&passwd, auth.owner_uid, auth.owner_gid)?;
    let controller = unit_name(&user, ServiceComponent::Controller)?;
    let worker = unit_name(&user, ServiceComponent::Worker)?;
    let mut action_evidence = Value::Null;
    if !matches!(
        action,
        BoundedServiceAction::InstallPinned | BoundedServiceAction::Query
    ) {
        ownership::verify(auth, &user)?;
    }
    match action {
        BoundedServiceAction::InstallPinned => {
            if fixture::config_path(&user).exists()
                || fixture::config_path(&user)
                    .with_extension("profiles.toml")
                    .exists()
            {
                return Err(failure(
                    "existing bounded service configuration belongs to the administrator",
                ));
            }
            // Never replace an existing service owned by another campaign or administrator.
            for unit in [&controller, &worker] {
                let loaded = systemctl(&["show", "--property=LoadState", "--value", unit])?;
                if loaded.trim() != "not-found" {
                    return Err(failure(
                        "bounded service already exists; preserve it instead of overwriting",
                    ));
                }
            }
            let binary = pin_binary(auth)?;
            let output = Command::new(&binary)
                .args([
                    "service",
                    "install",
                    "--mode",
                    "system-bounded-autotune",
                    "--controller-user",
                    &user,
                    "--unit-dir",
                    "/run/systemd/system",
                    "--config-dir",
                    "/run/stutter-bounded-service-validation",
                    "--binary",
                ])
                .arg(&binary)
                .env("PATH", ROOT_PATH)
                .output()
                .map_err(|error| failure(error.to_string()))?;
            if !output.status.success() {
                return Err(failure(format!(
                    "pinned service install failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )));
            }
            systemctl(&["daemon-reload"])?;
            ownership::record(auth, &user)?;
        }
        BoundedServiceAction::UninstallPinned => {
            systemctl(&["stop", &controller, &worker])?;
            let tokens = worker_tokens(&user);
            if tokens["readable"] != true || tokens["outstanding"] != 0 {
                return Err(failure(
                    "preserve installed service until its root-issued tokens are settled",
                ));
            }
            // Keep persistent product state, root code and the ownership receipt for audit.
            for path in ownership::paths(&user) {
                fs::remove_file(path).map_err(|error| failure(error.to_string()))?;
            }
            systemctl(&["daemon-reload"])?;
        }
        BoundedServiceAction::Start => {
            systemctl(&["start", &controller])?;
        }
        BoundedServiceAction::Stop => {
            systemctl(&["stop", &controller, &worker])?;
        }
        BoundedServiceAction::StopController => {
            systemctl(&["stop", &controller])?;
        }
        BoundedServiceAction::Restart => {
            systemctl(&["restart", &controller])?;
        }
        BoundedServiceAction::RestartWorker => {
            systemctl(&["restart", &worker])?;
        }
        BoundedServiceAction::Signal { component, signal } => {
            let unit = unit_name(&user, component)?;
            let signal = match signal {
                ServiceSignal::Term => "--signal=SIGTERM",
                ServiceSignal::Int => "--signal=SIGINT",
                ServiceSignal::Kill => "--signal=SIGKILL",
            };
            systemctl(&["kill", "--kill-whom=main", signal, &unit])?;
        }
        BoundedServiceAction::Query => (),
        BoundedServiceAction::SuspendResume => {
            let successes = || -> Result<u64, Denial> {
                fs::read_to_string("/sys/power/suspend_stats/success")
                    .map_err(|error| failure(error.to_string()))?
                    .trim()
                    .parse()
                    .map_err(|error: std::num::ParseIntError| failure(error.to_string()))
            };
            let before = successes()?;
            // The service manager executes this one fixed hardware action. The validation
            // daemon keeps its bounded capability set and read-only /etc namespace.
            let output = Command::new("systemd-run")
                .arg(format!(
                    "--unit=stutter-validation-suspend-{}",
                    auth.campaign_id
                ))
                .args([
                    "--wait",
                    "--collect",
                    "--property=Type=exec",
                    "--property=RuntimeMaxSec=90",
                    "--property=TimeoutStopSec=10",
                    "--",
                    "/run/current-system/sw/bin/rtcwake",
                    "-m",
                    "mem",
                    "-s",
                    "30",
                ])
                .env("PATH", ROOT_PATH)
                .output()
                .map_err(|error| failure(error.to_string()))?;
            let after = successes()?;
            if !output.status.success() || after <= before {
                return Err(failure(
                    "fixed suspend/resume did not increment the kernel success counter",
                ));
            }
            action_evidence = json!({"suspend_success_before":before,"suspend_success_after":after,"rtc_wake_seconds":30});
        }
        BoundedServiceAction::PrepareFixture {
            source_cpu,
            destination_cpu,
            operations,
        } => {
            fixture::prepare(auth, &user, source_cpu, destination_cpu, operations)?;
        }
        BoundedServiceAction::AttachFixture {
            pid,
            starttime_ticks,
        } => {
            fixture::attach(auth, &user, pid, starttime_ticks)?;
        }
    }
    let properties = "--property=ActiveState,SubState,MainPID,User,NoNewPrivileges,AmbientCapabilities,CapabilityBoundingSet,FragmentPath,ExecStart,ExecMainStartTimestampMonotonic,ExecMainStatus,ExecMainCode,Result,NRestarts,ControlGroup";
    let controller_state = systemctl(&["show", properties, &controller])?;
    let worker_state = systemctl(&["show", properties, &worker])?;
    let socket = Path::new("/run").join(format!("stutter-worker-{user}/worker.sock"));
    let socket_metadata = fs::symlink_metadata(&socket).ok().map(|metadata| {
        use std::os::unix::fs::FileTypeExt;
        json!({"uid": metadata.uid(), "mode": metadata.mode() & 0o777, "is_socket": metadata.file_type().is_socket()})
    });
    let tokens = worker_tokens(&user);
    let ownership_verified = ownership::verify(auth, &user).is_ok();
    let pinned = ownership_verified.then(|| ownership::binary(auth));
    Ok(
        json!({"controller_unit": controller, "worker_unit": worker, "action_evidence":action_evidence, "ownership_verified":ownership_verified,
        "controller_process": process_snapshot(&controller_state, pinned.as_deref(), &auth.stutter_sha256), "worker_process": process_snapshot(&worker_state, pinned.as_deref(), &auth.stutter_sha256),
        "controller": controller_state, "worker": worker_state, "socket": socket_metadata,
        "worker_tokens": tokens, "boot_id": stutter_core::boot::current_boot_id().map_err(|error| failure(error.to_string()))?}),
    )
}

fn worker_tokens(user: &str) -> Value {
    let ledger = Path::new("/var/lib/stutter").join(format!("worker-{user}/issued-tokens.json"));
    let mut summary = match fs::read(&ledger) {
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(Value::Array(entries)) => {
                // A token is told apart from every later one by its plan's creation time.
                let ids = |settled: bool| -> Vec<&str> {
                    entries
                        .iter()
                        .filter(|entry| (entry["restored"] == true) == settled)
                        .filter_map(|entry| entry["plan"]["created_unix_nanos"].as_str())
                        .collect()
                };
                json!({"readable": true,
                    "outstanding": entries.iter().filter(|entry| entry["restored"] != true).count(),
                    "outstanding_ids": ids(false), "settled_ids": ids(true)})
            }
            _ => json!({"readable": false, "error": "worker ledger is malformed"}),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            json!({"readable": true, "outstanding": 0})
        }
        Err(error) => json!({"readable": false, "error": error.to_string()}),
    };
    let audit = ledger.with_file_name("token-recovery-audit.jsonl");
    summary["recovery_audit"] = match fs::read_to_string(audit) {
        Ok(text) => {
            let entries = text
                .lines()
                .map(serde_json::from_str::<Value>)
                .collect::<Result<Vec<_>, _>>();
            match entries {
                Ok(entries) => {
                    json!({"readable":true,"completed":entries.iter().filter(|entry| entry["success"] == true).count(),
                        "refused":entries.iter().filter(|entry| entry["success"] == false).count(),
                        "dispositions":disposition_totals(&entries)})
                }
                Err(_) => json!({"readable":false,"error":"malformed root token recovery audit"}),
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            json!({"readable":true,"completed":0})
        }
        Err(error) => json!({"readable":false,"error":error.to_string()}),
    };
    summary
}

fn process_snapshot(properties: &str, pinned: Option<&Path>, expected_sha256: &str) -> Value {
    let pid = properties
        .lines()
        .find_map(|line| line.strip_prefix("MainPID="))
        .and_then(|pid| pid.parse::<u32>().ok())
        .filter(|pid| *pid > 0);
    let Some(pid) = pid else {
        return Value::Null;
    };
    let proc = Path::new("/proc").join(pid.to_string());
    let ticks = || {
        fs::read_to_string(proc.join("stat")).ok().and_then(|stat| {
            stat.rsplit_once(") ")
                .and_then(|(_, fields)| fields.split_whitespace().nth(19))
                .and_then(|field| field.parse::<u64>().ok())
        })
    };
    let before = ticks();
    let status = fs::read_to_string(proc.join("status")).ok();
    let executable_identity = || {
        fs::metadata(proc.join("exe"))
            .ok()
            .map(|metadata| (metadata.dev(), metadata.ino()))
    };
    let executable_before = executable_identity();
    // The ownership verifier hashed the protected installed file. Match the actual /proc
    // executable to that exact inode, avoiding two more whole-binary hashes per query.
    let pinned_identity = pinned
        .and_then(|path| fs::metadata(path).ok())
        .map(|metadata| (metadata.dev(), metadata.ino()));
    let binary = (executable_before.is_some() && executable_before == pinned_identity)
        .then(|| expected_sha256.to_owned());
    let after = ticks();
    let executable_after = executable_identity();
    let fields: std::collections::BTreeMap<_, _> = status
        .as_deref()
        .unwrap_or("")
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            ["Uid", "Gid", "CapEff", "CapBnd", "CapAmb", "NoNewPrivs"]
                .contains(&key)
                .then(|| (key.to_owned(), value.trim().to_owned()))
        })
        .collect();
    json!({"pid": pid, "starttime_ticks": before, "consistent": before.is_some() && before == after && executable_before.is_some() && executable_before == executable_after,
        "exe_dev":executable_before.map(|identity| identity.0), "exe_ino":executable_before.map(|identity| identity.1), "binary_sha256": binary, "status": fields})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn service_targets_come_only_from_the_authorized_non_root_account() {
        let passwd = "root:x:0:0::/root:/bin/sh\ndesktop:x:1000:100::/home/desktop:/bin/sh\n";
        assert_eq!(account_name(passwd, 1000, 100).unwrap(), "desktop");
        assert!(account_name(passwd, 0, 0).is_err());
        assert!(account_name(passwd, 1000, 101).is_err());
        assert_eq!(
            unit_name("desktop", ServiceComponent::Controller).unwrap(),
            "stutter-autotune-bounded@desktop.service"
        );
        assert!(unit_name("../other", ServiceComponent::Worker).is_err());
        assert!(
            serde_json::from_str::<BoundedServiceAction>(
                r#"{"signal":{"component":"controller","signal":"kill","unit":"sshd.service"}}"#
            )
            .is_err()
        );
    }
}

/// Totals of the per-task rollback dispositions the root worker wrote into its audit
/// (`disposition='restored=7 superseded_externally=1 ...'`), over successful and refused entries.
fn disposition_totals(entries: &[Value]) -> Value {
    let mut totals = std::collections::BTreeMap::<String, u64>::new();
    for entry in entries {
        let Some(message) = entry["message"].as_str() else {
            continue;
        };
        let Some(start) = message.find("disposition='") else {
            continue;
        };
        let rest = &message[start + "disposition='".len()..];
        for pair in rest.split('\'').next().unwrap_or("").split_whitespace() {
            if let Some((key, value)) = pair.split_once('=')
                && let Ok(value) = value.parse::<u64>()
            {
                *totals.entry(key.to_owned()).or_default() += value;
            }
        }
    }
    json!(totals)
}

#[cfg(test)]
mod disposition_tests {
    use super::*;

    #[test]
    fn disposition_totals_sum_across_entries() {
        let entries = vec![
            json!({"message":"worker-issued token settled digest=a disposition='restored=7 superseded_externally=1 unresolved=0'"}),
            json!({"message":"worker-issued token left unresolved digest=b disposition='restored=0 superseded_externally=2 unresolved=3'"}),
            json!({"message":"worker-issued token voided digest=c"}),
        ];
        let totals = disposition_totals(&entries);
        assert_eq!(totals["restored"], 7);
        assert_eq!(totals["superseded_externally"], 3);
        assert_eq!(totals["unresolved"], 3);
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;

    #[test]
    fn a_source_that_differs_from_the_authorized_digest_is_never_snapshotted() {
        let dir = std::env::temp_dir().join(format!("snapshot-source-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let source = dir.join("xtask");
        fs::write(&source, b"authorized bytes").unwrap();
        let digest = sha256_hex(b"authorized bytes");
        assert_eq!(
            read_authorized(&source, &digest, "xtask").unwrap(),
            b"authorized bytes"
        );
        // The user swaps the file after authorization: the snapshot is refused outright.
        fs::write(&source, b"swapped bytes").unwrap();
        assert!(read_authorized(&source, &digest, "xtask").is_err());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_snapshot_holds_exactly_the_hashed_bytes_read_only_and_ignores_later_source_edits() {
        let dir = std::env::temp_dir().join(format!("snapshot-write-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source");
        fs::write(&source, b"hashed bytes").unwrap();
        let bytes = read_authorized(&source, &sha256_hex(b"hashed bytes"), "x").unwrap();
        let snapshot = write_protected(&dir, "xtask", &bytes).unwrap();
        // Replacing the user's file afterwards cannot change what the root copy executes.
        fs::write(&source, b"replacement").unwrap();
        assert_eq!(fs::read(&snapshot).unwrap(), b"hashed bytes");
        assert_eq!(
            fs::metadata(&snapshot).unwrap().permissions().mode() & 0o777,
            0o555
        );
        fs::remove_dir_all(dir).ok();
    }
}
