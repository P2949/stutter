//! Measures the delivery bound of a renderer-stamped frame stream on a controlled fixture.
//!
//! A producer thread stands in for an instrumented renderer: it presents frames at a target rate
//! against absolute `CLOCK_MONOTONIC` deadlines, stamps each at its present call and pushes it to
//! the controller over a socket. The controller reads the same clock the instant each frame
//! arrives, validates it and records the delivery latency; the calibration is the measured bound.
//! Optional CPU-spinning threads add load. This measures the *method*: the fixture is not a game
//! and not a Vulkan layer, and it claims no attribution. The 2 ms gate is applied unchanged and
//! the result is reported whatever it is.
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::{fs::OpenOptionsExt, net::UnixStream},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, ensure};
use clap::Args;
use stutter::api::{
    objective_source::monotonic_now_ns,
    telemetry::{MAX_ALIGNMENT_UNCERTAINTY_MS, StampedFrame, StampedStream},
};

#[derive(Args, Clone, Debug, Eq, PartialEq)]
pub struct FrameClockArgs {
    /// Frames to present (1000..=20000; fewer cannot calibrate).
    #[arg(long, default_value_t = 3000)]
    pub frames: u64,
    /// Target frame rate (10..=240).
    #[arg(long, default_value_t = 60)]
    pub hz: u64,
    /// CPU-spinning threads that compete with the fixture (0..=16).
    #[arg(long, default_value_t = 0)]
    pub spin_threads: u32,
    /// New file for the JSON result.
    #[arg(long)]
    pub out: PathBuf,
}

fn producer(mut stream: UnixStream, frames: u64, period: Duration) -> anyhow::Result<()> {
    let start = Instant::now() + Duration::from_millis(200);
    for seq in 0..frames {
        let deadline = start + period * u32::try_from(seq).context("frame index")?;
        if let Some(wait) = deadline.checked_duration_since(Instant::now()) {
            thread::sleep(wait);
        }
        // The stamp is the frame event itself, taken at the present call.
        let stamp = monotonic_now_ns()?;
        writeln!(stream, "{{\"seq\":{seq},\"monotonic_ns\":{stamp}}}")?;
    }
    Ok(())
}

pub fn run(args: &FrameClockArgs) -> anyhow::Result<()> {
    ensure!(
        (1000..=20_000).contains(&args.frames),
        "frames must be 1000..=20000"
    );
    ensure!((10..=240).contains(&args.hz), "hz must be 10..=240");
    ensure!(args.spin_threads <= 16, "spin threads must be 0..=16");
    let out = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&args.out)
        .context("the result file must be new")?;
    let (producer_end, controller_end) = UnixStream::pair()?;
    let stop = Arc::new(AtomicBool::new(false));
    let spinners: Vec<_> = (0..args.spin_threads)
        .map(|_| {
            let stop = Arc::clone(&stop);
            thread::spawn(move || {
                let mut value = 1_u64;
                while !stop.load(Ordering::Relaxed) {
                    value = std::hint::black_box(
                        value.wrapping_mul(6364136223846793005).wrapping_add(1),
                    );
                }
                value
            })
        })
        .collect();
    let period = Duration::from_nanos(1_000_000_000 / args.hz);
    let frames = args.frames;
    let renderer = thread::spawn(move || producer(producer_end, frames, period));

    let mut stream = StampedStream::default();
    let mut reader = BufReader::new(controller_end);
    let mut line = String::new();
    let mut malformed = 0_u64;
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        // Read the shared clock before anything else happens to the line.
        let received = monotonic_now_ns()?;
        match serde_json::from_str::<StampedFrame>(&line) {
            Ok(frame) => {
                let _ = stream.push(frame, received);
            }
            Err(_) => malformed += 1,
        }
    }
    renderer
        .join()
        .map_err(|_| anyhow::anyhow!("the fixture renderer panicked"))??;
    stop.store(true, Ordering::Relaxed);
    for spinner in spinners {
        let _ = spinner.join();
    }
    let calibration = stream
        .calibration()
        .context("not enough accepted frames to calibrate")?;
    let uncertainty_ms = calibration.uncertainty_ms();
    let load = fs::read_to_string("/proc/loadavg").unwrap_or_default();
    let result = serde_json::json!({
        "schema_version": 1,
        "evidence_kind": "controlled_fixture",
        "scope": "delivery-latency bound of a renderer-stamped CLOCK_MONOTONIC stream on a synthetic fixture in one process and time namespace; not a game, not a Vulkan layer, no attribution established",
        "frames_requested": args.frames,
        "hz": args.hz,
        "spin_threads": args.spin_threads,
        "accepted": stream.accepted(),
        "rejected": stream.rejected(),
        "malformed": malformed,
        "unexplained_gaps": stream.unexplained_gaps(),
        "calibration": {
            "frames": calibration.frames,
            "max_ns": calibration.max_ns,
            "p99_ns": calibration.p99_ns,
            "p999_ns": calibration.p999_ns,
            "mean_ns": calibration.mean_ns,
        },
        "alignment_uncertainty_ms": uncertainty_ms,
        "gate_ms": MAX_ALIGNMENT_UNCERTAINTY_MS,
        "alignment_gate_passed": uncertainty_ms <= MAX_ALIGNMENT_UNCERTAINTY_MS
            && stream.rejected() == 0
            && stream.unexplained_gaps() == 0
            && malformed == 0,
        "loadavg": load.trim(),
    });
    let mut out = out;
    out.write_all(serde_json::to_string_pretty(&result)?.as_bytes())?;
    out.sync_all()?;
    println!(
        "frame-clock-prototype: {} frames, uncertainty {uncertainty_ms:.3} ms (gate {} ms), gate_passed={}",
        calibration.frames, MAX_ALIGNMENT_UNCERTAINTY_MS, result["alignment_gate_passed"]
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    fn args(extra: &[&str]) -> Result<FrameClockArgs, String> {
        let mut argv = vec!["xtask", "frame-clock-prototype", "--out", "result.json"];
        argv.extend_from_slice(extra);
        match crate::Cli::try_parse_from(argv) {
            Ok(cli) => match cli.command {
                crate::XtaskCommand::FrameClockPrototype(args) => Ok(args),
                _ => Err("wrong command".into()),
            },
            Err(error) => Err(error.to_string()),
        }
    }

    #[test]
    fn the_command_requires_an_output_file_and_has_bounded_defaults() {
        assert!(crate::Cli::try_parse_from(["xtask", "frame-clock-prototype"]).is_err());
        let defaults = args(&[]).unwrap();
        assert_eq!(
            (defaults.frames, defaults.hz, defaults.spin_threads),
            (3000, 60, 0)
        );
    }

    #[test]
    fn out_of_range_runs_are_refused_before_anything_starts() {
        let dir = std::env::temp_dir().join(format!("stutter-fcp-{}", std::process::id()));
        let _ = fs::remove_file(&dir);
        for (frames, hz, spin) in [
            (999, 60, 0),
            (20_001, 60, 0),
            (3000, 9, 0),
            (3000, 241, 0),
            (3000, 60, 17),
        ] {
            let bad = FrameClockArgs {
                frames,
                hz,
                spin_threads: spin,
                out: dir.clone(),
            };
            assert!(run(&bad).is_err(), "{frames} {hz} {spin}");
        }
        assert!(!dir.exists(), "a refused run created its result file");
    }

    #[test]
    fn an_existing_result_file_is_never_overwritten() {
        let path = std::env::temp_dir().join(format!("stutter-fcp-exists-{}", std::process::id()));
        fs::write(&path, "keep").unwrap();
        let run_args = FrameClockArgs {
            frames: 1000,
            hz: 240,
            spin_threads: 0,
            out: path.clone(),
        };
        assert!(run(&run_args).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "keep");
        let _ = fs::remove_file(path);
    }
}
