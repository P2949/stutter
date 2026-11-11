//! Binds a REAL instrumented renderer to the authenticated stamped source and records what the
//! controller can and cannot conclude from it.
//!
//! The renderer is an unmodified Vulkan application (`vkcube`) loaded with the opt-in
//! `VK_LAYER_STUTTER_present` layer (`packaging/vulkan-layer`). The controller listens on an
//! owner-only socket in a fresh `0700` directory, authenticates the connecting process by the
//! kernel's peer credentials and its start time, requires the shared time namespace, and validates
//! every frame line. The event is the application's present SUBMISSION, never scan-out, and with no
//! independent stamp-accuracy reference the alignment uncertainty stays unknown, so direct-frame
//! keep remains denied. A wrong-process policy is also run against the same connection to show the
//! refusal.
use std::{
    fs,
    io::{BufRead, BufReader},
    os::unix::{
        fs::{DirBuilderExt, OpenOptionsExt},
        net::UnixListener,
    },
    path::PathBuf,
    process::{Command, Stdio},
};

use anyhow::{Context, ensure};
use clap::Args;
use stutter::api::{
    objective_source::monotonic_now_ns,
    telemetry::{SourcePolicy, StampedSource, live_process},
};

#[derive(Args, Clone, Debug, Eq, PartialEq)]
pub struct RendererProducerArgs {
    /// Directory holding `libstutter_present_layer.so` and its manifest (`VK_LAYER_STUTTER_present.json`).
    #[arg(long)]
    pub layer_dir: PathBuf,
    /// Frames the renderer presents before it exits (1200..=6000).
    #[arg(long, default_value_t = 300)]
    pub frames: u32,
    /// New file for the JSON result.
    #[arg(long)]
    pub out: PathBuf,
}

pub fn run(args: &RendererProducerArgs) -> anyhow::Result<()> {
    ensure!(
        (1200..=6000).contains(&args.frames),
        "frames must be 1200..=6000"
    );
    let out = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&args.out)
        .context("the result file must be new")?;
    let dir = std::env::temp_dir().join(format!("stutter-renderer-{}", std::process::id()));
    fs::DirBuilder::new().mode(0o700).create(&dir)?;
    let socket = dir.join("frames.sock");
    let listener = UnixListener::bind(&socket)?;
    let mut renderer = Command::new("vkcube")
        .args(["--c", &args.frames.to_string(), "--suppress_popups"])
        .env("STUTTER_FRAME_SOCKET", &socket)
        .env("VK_ADD_LAYER_PATH", &args.layer_dir)
        .env("VK_INSTANCE_LAYERS", "VK_LAYER_STUTTER_present")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("start the instrumented renderer (vkcube)")?;
    let result = observe(&listener, &renderer, u64::from(args.frames) / 2);
    let _ = renderer.kill();
    let _ = renderer.wait();
    let _ = fs::remove_dir_all(&dir);
    let result = result?;
    serde_json::to_writer_pretty(out, &result)?;
    Ok(())
}

fn observe(
    listener: &UnixListener,
    renderer: &std::process::Child,
    sample_at: u64,
) -> anyhow::Result<serde_json::Value> {
    let pid = renderer.id();
    let (socket, _) = listener.accept().context("the renderer never connected")?;
    let start_ticks = live_process(std::path::Path::new("/proc"), pid)
        .context("the renderer is not alive")?
        .start_ticks;
    let policy = |renderer_pid: u32| SourcePolicy {
        owner_uid: stutter::api::telemetry::kernel_peer_credentials(&socket)
            .map_or(u32::MAX, |peer| peer.uid),
        renderer_pid,
        renderer_start_ticks: start_ticks,
        observer_pid: std::process::id(),
        proc_root: PathBuf::from("/proc"),
    };
    let mut reader = BufReader::new(socket.try_clone()?);
    let mut hello = String::new();
    reader.read_line(&mut hello)?;
    let mut stranger = StampedSource::new(policy(pid.wrapping_add(1)));
    let wrong_process = format!("{:?}", stranger.accept(&socket, &hello));
    let mut source = StampedSource::new(policy(pid));
    let bind = source.accept(&socket, &hello);
    ensure!(bind.is_ok(), "the real renderer did not bind: {bind:?}");
    let (mut malformed, mut rejected, mut seen) = (0_u64, 0_u64, 0_u64);
    let mut live: Option<(serde_json::Value, Option<serde_json::Value>)> = None;
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let received = monotonic_now_ns()?;
        match source.ingest(&line, received) {
            Ok(_) => {}
            Err(stutter::api::telemetry::IngestError::Rejected(_)) => rejected += 1,
            Err(_) => malformed += 1,
        }
        seen += 1;
        if seen == sample_at {
            // Sampled while the renderer is alive: its identity is rechecked at this moment.
            let now = monotonic_now_ns()?;
            let quality = source.quality_at(now);
            let calibration = source.stream().calibration().map(|c| serde_json::json!({
                "frames": c.frames, "max_ns": c.max_ns, "p99_ns": c.p99_ns, "mean_ns": c.mean_ns,
            }));
            live = Some((
                serde_json::json!({
                    "quality": quality,
                    "direct_frame_keep": quality.frame_primary_eligible().map_or_else(
                        |denial| format!("denied: {denial:?}"),
                        |()| "eligible".to_owned(),
                    ),
                }),
                calibration,
            ));
        }
    }
    let quality = source.quality_at(monotonic_now_ns()?);
    let calibration = source.stream().calibration();
    Ok(serde_json::json!({
        "schema_version": 1,
        "evidence_kind": "real_instrumented_renderer",
        "renderer": "vkcube (unmodified Vulkan application) with VK_LAYER_STUTTER_present",
        "event": "application present submission (vkQueuePresentKHR entry); not scan-out",
        "renderer_pid": pid,
        "renderer_start_ticks": start_ticks,
        "bound_by": "kernel peer credentials, process start time, shared monotonic time namespace, hello epoch",
        "wrong_process_policy_result": wrong_process,
        "accepted": source.stream().accepted(),
        "rejected": rejected,
        "malformed": malformed,
        "unexplained_gaps": source.stream().unexplained_gaps(),
        "calibration": calibration.map(|c| serde_json::json!({
            "frames": c.frames, "max_ns": c.max_ns, "p99_ns": c.p99_ns, "mean_ns": c.mean_ns,
        })),
        "while_running_midstream": live.as_ref().map(|(sampled, _)| sampled),
        "delivery_while_running": live.as_ref().and_then(|(_, delivery)| delivery.as_ref()),
        "quality_after_renderer_exit": quality,
        "stamp_accuracy_reference": "none: no photodiode, camera or display hardware timestamp exists on this host",
    }))
}
