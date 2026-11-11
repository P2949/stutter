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

pub async fn run(input: ResearchClientInput) -> anyhow::Result<()> {
    let uid = crate::syscall::geteuid();
    anyhow::ensure!(uid != 0, "research client must be unprivileged");
    if let ResearchClientAction::Inventory {
        steamapps,
        proton_manifest,
    } = &input.action
    {
        let game = crate::autotune::research::inspect_cyberpunk_installation(steamapps, uid)?;
        let proton = proton_manifest
            .as_deref()
            .map(|path| crate::autotune::research::inspect_proton_inventory(path, steamapps, uid))
            .transpose()?;
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
    let directory = crate::syscall::open_dir_no_symlinks(&socket_dir)?;
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
    serde_json::to_writer(&mut stream, &request)?;
    stream.write_all(b"\n")?;
    let line =
        crate::autotune::research::read_control_frame(&stream, 4096, Duration::from_secs(5))?;
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
