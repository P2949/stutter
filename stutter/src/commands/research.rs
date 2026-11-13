//! One-shot, unprivileged client for the resident research-control socket.
use std::{
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsFd, AsRawFd},
        unix::fs::{FileTypeExt, MetadataExt},
    },
    path::PathBuf,
    time::Duration,
};

use crate::autotune::research::{ResearchIntentV1, ResearchRequest};

#[derive(Debug)]
pub enum ResearchClientAction {
    Inventory {
        steamapps: PathBuf,
        proton_manifest: Option<PathBuf>,
    },
    Submit(PathBuf),
    /// Steam-invoked compatibility-tool entry; replaces this process with the planned Proton.
    Dispatch {
        context_dir: PathBuf,
        args: Vec<String>,
    },
    Status,
    Pause,
    Resume,
    Cancel,
    Report,
    Verify,
}

#[derive(Debug)]
pub struct ResearchClientInput {
    pub socket_dir: Option<PathBuf>,
    pub action: ResearchClientAction,
}

fn request(action: ResearchClientAction, uid: u32) -> anyhow::Result<ResearchRequest> {
    Ok(match action {
        ResearchClientAction::Dispatch { .. } => {
            anyhow::bail!("dispatch does not use the resident socket")
        }
        ResearchClientAction::Inventory { .. } => {
            anyhow::bail!("inventory does not use the resident socket")
        }
        ResearchClientAction::Submit(path) => {
            let parent = path
                .parent()
                .ok_or_else(|| anyhow::anyhow!("intent has no parent directory"))?;
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| anyhow::anyhow!("invalid intent filename"))?;
            anyhow::ensure!(
                name != "." && name != ".." && !name.contains('/'),
                "invalid intent filename"
            );
            let directory = crate::syscall::open_dir_no_symlinks(parent)?;
            let dir_meta = File::from(directory.try_clone()?).metadata()?;
            anyhow::ensure!(
                dir_meta.is_dir() && dir_meta.uid() == uid && dir_meta.mode() & 0o077 == 0,
                "untrusted intent directory"
            );
            let file = crate::syscall::openat_nofollow(directory.as_fd(), name)?;
            let meta = file.metadata()?;
            anyhow::ensure!(
                meta.is_file()
                    && meta.nlink() == 1
                    && meta.uid() == uid
                    && meta.mode() & 0o077 == 0
                    && meta.len() <= 16 * 1024,
                "intent must be a private, user-owned regular file of at most 16 KiB"
            );
            let mut bytes = Vec::new();
            file.take(16 * 1024 + 1).read_to_end(&mut bytes)?;
            anyhow::ensure!(bytes.len() <= 16 * 1024, "intent exceeds size limit");
            let intent: ResearchIntentV1 = serde_json::from_slice(&bytes)?;
            intent.validate()?;
            anyhow::ensure!(intent.uid == uid, "intent belongs to another user");
            ResearchRequest::Submit {
                intent: Box::new(intent),
            }
        }
        ResearchClientAction::Status => ResearchRequest::Status {},
        ResearchClientAction::Pause => ResearchRequest::Pause {},
        ResearchClientAction::Resume => ResearchRequest::Resume {},
        ResearchClientAction::Cancel => ResearchRequest::Cancel {},
        ResearchClientAction::Report => ResearchRequest::Report {},
        ResearchClientAction::Verify => ResearchRequest::Verify {},
    })
}

/// Authorize Steam's call against the resident-armed context, then exec the real Proton. Only
/// returns on failure. Nothing here talks to the resident or touches root.
fn dispatch(context_dir: &std::path::Path, args: &[String], uid: u32) -> anyhow::Result<()> {
    use std::os::unix::process::CommandExt;
    let root = File::from(crate::syscall::open_dir_no_symlinks(context_dir)?);
    let meta = root.metadata()?;
    anyhow::ensure!(
        meta.is_dir() && meta.uid() == uid && meta.mode() & 0o077 == 0,
        "untrusted dispatch directory"
    );
    let directory = crate::autotune::research::dispatch_context::latest_attempt_dir(&root)?;
    let attempt = directory.metadata()?;
    anyhow::ensure!(
        attempt.is_dir() && attempt.uid() == uid && attempt.mode() & 0o077 == 0,
        "untrusted dispatch attempt directory"
    );
    let env: std::collections::BTreeMap<String, String> = std::env::vars().collect();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let (plan, ticket) = crate::autotune::research::dispatch_context::authorize_dispatch(
        &directory,
        args,
        &env,
        uid,
        now,
        &|context, tool| {
            crate::autotune::research::dispatch_context::verify_installed_tool(context, tool, uid)
        },
    )?;
    crate::autotune::research::dispatch_context::record_dispatcher_start(&directory, &ticket)?;
    let error = std::process::Command::new(&plan.program)
        .args(&plan.args)
        .env_clear()
        .envs(&plan.env)
        .exec();
    Err(anyhow::Error::new(error).context("failed to exec the authorized Proton"))
}

pub async fn run(input: ResearchClientInput) -> anyhow::Result<()> {
    let uid = crate::syscall::geteuid();
    anyhow::ensure!(uid != 0, "research client must be unprivileged");
    if let ResearchClientAction::Dispatch { context_dir, args } = &input.action {
        return dispatch(context_dir, args, uid);
    }
    if let ResearchClientAction::Inventory {
        steamapps,
        proton_manifest,
    } = &input.action
    {
        let libraries = crate::autotune::research::discover_steam_libraries(steamapps, uid)?;
        let (_, game) =
            crate::autotune::research::discover_cyberpunk_installation(&libraries, uid)?;
        let proton = proton_manifest
            .as_deref()
            .map(|path| {
                crate::autotune::research::inspect_proton_inventory(
                    path, steamapps, &libraries, uid,
                )
            })
            .transpose()?;
        anyhow::ensure!(
            crate::autotune::research::discover_steam_libraries(steamapps, uid)? == libraries,
            "Steam libraries changed during inventory"
        );
        let inventory = serde_json::json!({
            "schema_version": 1,
            "cyberpunk": game,
            "proton": proton,
            "proton_eligible_count": proton.as_ref().map_or(0, |value| value.tools().len()),
        });
        println!("{}", serde_json::to_string(&inventory)?);
        return Ok(());
    }
    let request = request(input.action, uid)?;
    let socket_dir = input
        .socket_dir
        .ok_or_else(|| anyhow::anyhow!("--socket-dir is required for resident requests"))?;
    let retry_submit = matches!(&request, ResearchRequest::Submit { .. });
    let result = exchange_once(&socket_dir, uid, request.clone()).await;
    if retry_submit
        && result
            .as_ref()
            .is_err_and(|error| error.is::<SubmissionAcknowledgementUnknown>())
    {
        return exchange_once(&socket_dir, uid, request)
            .await
            .map_err(|error| {
                error.context(
                    "the original submission may have been admitted; inspect campaign status",
                )
            });
    }
    result
}

async fn exchange_once(
    socket_dir: &std::path::Path,
    uid: u32,
    request: ResearchRequest,
) -> anyhow::Result<()> {
    let directory = crate::syscall::open_dir_no_symlinks(socket_dir)?;
    let dir_meta = File::from(directory.try_clone()?).metadata()?;
    anyhow::ensure!(
        dir_meta.is_dir() && dir_meta.uid() == uid && dir_meta.mode() & 0o077 == 0,
        "untrusted research socket directory"
    );
    let path = PathBuf::from(format!(
        "/proc/self/fd/{}/research.sock",
        directory.as_raw_fd()
    ));
    let meta = std::fs::symlink_metadata(&path)?;
    anyhow::ensure!(
        meta.file_type().is_socket() && meta.uid() == uid && meta.mode() & 0o077 == 0,
        "untrusted research socket"
    );
    let stream = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::UnixStream::connect(path),
    )
    .await??;
    drop(directory);
    let stream = stream.into_std()?;
    tokio::task::spawn_blocking(move || exchange(stream, uid, request)).await?
}

#[derive(Debug, thiserror::Error)]
#[error("submission acknowledgement is unavailable; query status or retry the same intent")]
struct SubmissionAcknowledgementUnknown;

fn exchange(
    mut stream: std::os::unix::net::UnixStream,
    uid: u32,
    request: ResearchRequest,
) -> anyhow::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let (pid, peer_uid, _) = crate::syscall::unix_peer_credentials(&stream)?;
    let peer = crate::telemetry::proc_view::live_process(std::path::Path::new("/proc"), pid)
        .ok_or_else(|| anyhow::anyhow!("resident process unavailable"))?;
    anyhow::ensure!(
        peer_uid == uid && peer.uid == uid && peer.start_ticks != 0,
        "unexpected resident peer"
    );
    let submit = matches!(&request, ResearchRequest::Submit { .. });
    let mut transaction = || -> anyhow::Result<Vec<u8>> {
        serde_json::to_writer(&mut stream, &request)?;
        stream.write_all(b"\n")?;
        crate::autotune::research::read_control_frame(&stream, 4096, Duration::from_secs(5))
    };
    let line = match transaction() {
        Ok(line) => line,
        Err(_) if submit => return Err(SubmissionAcknowledgementUnknown.into()),
        Err(error) => return Err(error),
    };
    let live = crate::telemetry::proc_view::live_process(std::path::Path::new("/proc"), pid)
        .ok_or_else(|| anyhow::anyhow!("resident process exited"))?;
    anyhow::ensure!(
        live.uid == uid && live.start_ticks == peer.start_ticks,
        "resident process changed"
    );
    let response: serde_json::Value = serde_json::from_slice(&line)?;
    println!("{}", serde_json::to_string(&response)?);
    anyhow::ensure!(
        matches!(
            response.get("status").and_then(|value| value.as_str()),
            Some("idle" | "accepted" | "ok")
        ),
        "research request was denied or blocked"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::{PermissionsExt, symlink};

    use super::*;

    #[tokio::test]
    async fn lost_submission_acknowledgement_retries_the_identical_intent() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let socket = dir.path().join("research.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let uid = crate::syscall::geteuid();
        let intent = serde_json::json!({
            "schema_version": 1, "game": "cyberpunk2077", "app_id": 1091500,
            "uid": uid, "boot_id": "boot-1", "session_id": "11",
            "steam_client": {"branch":"publicbeta", "steamrt64_build_id":1,
                "steamrt64_manifest_sha256":"e".repeat(64), "installed_index_sha256":"f".repeat(64),
                "identity_sha256":"1".repeat(64), "process":{"steam_pid":10,
                "steam_start_ticks":1,"webhelper_pid":11,"webhelper_start_ticks":2,"logged_in":true}},
            "source_sha256":"a".repeat(64), "policy_sha256":"b".repeat(64),
            "game_executable_sha256":"c".repeat(64), "build_id":1,
            "tools":[{"source":"ge","identity_sha256":"d".repeat(64)}],
            "factors":[], "actions":[], "issued_unix_seconds":10, "deadline_unix_seconds":3610,
            "max_active_seconds":3600, "max_attempt_seconds":720,
            "max_owned_pressure_seconds_per_study":600, "max_attempts":10,
            "max_evidence_bytes":1024, "desktop_research_consented":true
        });
        let intent_path = dir.path().join("intent.json");
        std::fs::write(&intent_path, serde_json::to_vec(&intent).unwrap()).unwrap();
        std::fs::set_permissions(&intent_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let server = tokio::spawn(async move {
            let (first, _) = listener.accept().await.unwrap();
            let mut first = BufReader::new(first);
            let mut first_line = String::new();
            first.read_line(&mut first_line).await.unwrap();
            drop(first);
            let (second, _) = listener.accept().await.unwrap();
            let mut second = BufReader::new(second);
            let mut second_line = String::new();
            second.read_line(&mut second_line).await.unwrap();
            assert_eq!(first_line, second_line);
            second
                .get_mut()
                .write_all(b"{\"status\":\"ok\",\"phase\":\"await_safe_window\"}\n")
                .await
                .unwrap();
        });
        tokio::time::timeout(
            Duration::from_secs(10),
            run(ResearchClientInput {
                socket_dir: Some(dir.path().to_path_buf()),
                action: ResearchClientAction::Submit(intent_path),
            }),
        )
        .await
        .unwrap()
        .unwrap();
        server.await.unwrap();
    }

    #[test]
    fn submit_rejects_public_or_symlinked_intent_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("intent.json");
        std::fs::write(&path, b"{}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(
            request(
                ResearchClientAction::Submit(path.clone()),
                crate::syscall::geteuid()
            )
            .is_err()
        );
        let link = dir.path().join("link.json");
        symlink(&path, &link).unwrap();
        assert!(
            request(
                ResearchClientAction::Submit(link),
                crate::syscall::geteuid()
            )
            .is_err()
        );
    }
}
