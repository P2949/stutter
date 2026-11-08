//! Bounded, nonblocking KPI observation on the live controller's tick. No action API lives here.
use std::{
    io::{Read, Write},
    os::unix::{
        fs::MetadataExt,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use super::{
    KpiContract, SampleStore, WindowAggregate, denial_code, ingest::MAX_SAMPLE_LINE_BYTES,
};
use crate::telemetry::{proc_view::live_process, registration::PeerCredentials};

const TICK_BYTES: usize = 4096;
const TICK_TIME: Duration = Duration::from_millis(2);
const CONNECTION_IDLE: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KpiMetricObservation {
    pub metric: String,
    pub unit: String,
    pub role: String,
    pub aggregate: Option<WindowAggregate>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KpiObservation {
    pub contract_digest: String,
    pub source_session: String,
    pub binding: super::SourceBinding,
    pub status: String,
    pub accepted_samples: u64,
    pub denied_samples: u64,
    pub last_denial: Option<String>,
    pub metrics: Vec<KpiMetricObservation>,
    pub authorizes_mutation: bool,
}

#[derive(Debug)]
struct Connection {
    stream: UnixStream,
    peer: PeerCredentials,
    line: Vec<u8>,
    last_input: Instant,
}

#[derive(Debug)]
pub struct KpiCollector {
    listener: UnixListener,
    socket_path: PathBuf,
    socket_identity: (u64, u64),
    contract: KpiContract,
    digest: String,
    boot_id: String,
    source_session: String,
    started_ns: u64,
    executable_identity: (u64, u64),
    store: SampleStore,
    connection: Option<Connection>,
    context_active: bool,
    context_started_ns: u64,
    loss_since_ns: Option<u64>,
    accepted: u64,
    denied: u64,
    last_denial: Option<String>,
}

impl KpiCollector {
    pub fn contract(&self) -> &KpiContract {
        &self.contract
    }
    pub fn capture_block(
        &self,
        request: super::KpiBlockRequest,
    ) -> anyhow::Result<super::ObservedKpiBlock> {
        let binding = self.contract.binding;
        anyhow::ensure!(self.context_active, "collector has no active exact target");
        anyhow::ensure!(
            request.from_ns >= self.context_started_ns,
            "block crosses a target/source context boundary"
        );
        anyhow::ensure!(
            self.loss_since_ns.is_none_or(|lost| request.to_ns < lost),
            "source has unexplained loss; a fresh collector session is required"
        );
        anyhow::ensure!(
            self.connection
                .as_ref()
                .is_none_or(|connection| connection.line.is_empty()),
            "a partial source line is still pending"
        );
        anyhow::ensure!(
            live_process(Path::new("/proc"), binding.pid).is_some_and(|live| live.uid
                == binding.uid
                && live.start_ticks == binding.start_ticks),
            "producer exited or changed before capture"
        );
        anyhow::ensure!(
            crate::telemetry::identity::read_boot_id()? == self.boot_id,
            "boot context changed before capture"
        );
        let executable = std::fs::metadata(format!("/proc/{}/exe", binding.pid))?;
        anyhow::ensure!(
            (executable.dev(), executable.ino()) == self.executable_identity,
            "producer executable changed before capture"
        );
        let now_ns = crate::syscall::clock_gettime_ns(libc::CLOCK_MONOTONIC)?;
        anyhow::ensure!(request.to_ns <= now_ns, "block ends in the future");
        anyhow::ensure!(
            request.from_ns >= self.started_ns,
            "block begins before this collector session"
        );
        for spec in std::iter::once(&self.contract.primary).chain(&self.contract.hard_guardrails) {
            anyhow::ensure!(
                now_ns.saturating_sub(request.to_ns) <= spec.max_age_ms.saturating_mul(1_000_000),
                "block source is stale at capture"
            );
        }
        super::ObservedKpiBlock::capture(
            request,
            self.source_session.clone(),
            self.boot_id.clone(),
            &self.contract,
            &self.store,
        )
    }
    pub fn bind(contract: KpiContract, dir: &Path) -> anyhow::Result<Self> {
        contract
            .validate()
            .map_err(|error| anyhow::anyhow!("invalid KPI contract: {error:?}"))?;
        anyhow::ensure!(
            contract.binding.uid == crate::syscall::geteuid(),
            "KPI producer must belong to the monitored user"
        );
        let live = live_process(Path::new("/proc"), contract.binding.pid)
            .ok_or_else(|| anyhow::anyhow!("KPI producer is absent"))?;
        anyhow::ensure!(
            live.uid == contract.binding.uid && live.start_ticks == contract.binding.start_ticks,
            "KPI producer identity has changed"
        );
        let executable = std::fs::metadata(format!("/proc/{}/exe", contract.binding.pid))?;
        let boot_id = crate::telemetry::identity::read_boot_id()?;
        let listener =
            crate::telemetry::registration::bind_fresh(dir, "kpi.sock", contract.binding.uid)
                .map_err(|error| anyhow::anyhow!("cannot bind KPI observer: {error:?}"))?;
        listener.set_nonblocking(true)?;
        let socket_path = dir.join("kpi.sock");
        let metadata = std::fs::symlink_metadata(&socket_path)?;
        let digest = stutter_core::hash::sha256_hex(&serde_json::to_vec(&contract)?);
        Ok(Self {
            listener,
            socket_path,
            socket_identity: (metadata.dev(), metadata.ino()),
            digest,
            boot_id,
            source_session: crate::telemetry::identity::new_capture_id()?,
            started_ns: crate::syscall::clock_gettime_ns(libc::CLOCK_MONOTONIC)?,
            executable_identity: (executable.dev(), executable.ino()),
            store: SampleStore::new(contract.clone()),
            contract,
            connection: None,
            context_active: false,
            context_started_ns: 0,
            loss_since_ns: None,
            accepted: 0,
            denied: 0,
            last_denial: None,
        })
    }

    fn deny(&mut self, reason: &str) {
        self.denied = self.denied.saturating_add(1);
        self.last_denial = Some(reason.to_owned());
    }

    /// Only the exact currently resolved root may supply this observer. A focus/process change
    /// clears its samples, including when the old target later returns. This never supplies the
    /// objective signals used by the action manager or support registry.
    pub fn poll(
        &mut self,
        target: Option<(u32, u64)>,
        proc_root: &Path,
        now_ns: u64,
        window_ns: u64,
    ) -> KpiObservation {
        let binding = self.contract.binding;
        let active = target == Some((binding.pid, binding.start_ticks))
            && live_process(proc_root, binding.pid).is_some_and(|live| {
                live.uid == binding.uid && live.start_ticks == binding.start_ticks
            })
            && crate::telemetry::identity::read_boot_id().is_ok_and(|boot| boot == self.boot_id);
        let active = active
            && std::fs::metadata(proc_root.join(binding.pid.to_string()).join("exe"))
                .is_ok_and(|meta| (meta.dev(), meta.ino()) == self.executable_identity);
        if !active {
            self.store = SampleStore::new(self.contract.clone());
            self.context_active = false;
            self.connection = None;
            self.loss_since_ns = None;
            return self.observation("target_identity_mismatch", now_ns, window_ns);
        }
        if !self.context_active {
            self.context_started_ns = now_ns;
        }
        self.context_active = true;
        let deadline = Instant::now() + TICK_TIME;
        if self
            .connection
            .as_ref()
            .is_some_and(|connection| connection.last_input.elapsed() > CONNECTION_IDLE)
        {
            if self
                .connection
                .as_ref()
                .is_some_and(|connection| !connection.line.is_empty())
            {
                self.loss_since_ns.get_or_insert(now_ns);
            }
            self.connection = None;
            self.deny("connection_idle");
        }
        if self.connection.is_none()
            && let Ok((stream, _)) = self.listener.accept()
        {
            let credentials = crate::syscall::unix_peer_credentials(&stream);
            match credentials {
                Ok((pid, uid, _)) if pid == binding.pid && uid == binding.uid => {
                    if stream.set_nonblocking(true).is_ok() {
                        self.connection = Some(Connection {
                            stream,
                            peer: PeerCredentials { pid, uid },
                            line: Vec::with_capacity(MAX_SAMPLE_LINE_BYTES + 1),
                            last_input: Instant::now(),
                        });
                    }
                }
                _ => self.deny("not_the_bound_process"),
            }
        }
        let mut bytes = 0;
        while bytes < TICK_BYTES && Instant::now() < deadline {
            let Some(mut connection) = self.connection.take() else {
                break;
            };
            let mut byte = [0];
            match connection.stream.read(&mut byte) {
                Ok(0) => {
                    if !connection.line.is_empty() {
                        self.loss_since_ns.get_or_insert(now_ns);
                        self.deny("partial_line_at_eof");
                    }
                    break;
                }
                Ok(_) => {
                    bytes += 1;
                    connection.last_input = Instant::now();
                    if byte[0] == b'\n' {
                        // Re-read the live identity per line, never a client-supplied start time.
                        let ticks =
                            live_process(proc_root, binding.pid).map_or(0, |live| live.start_ticks);
                        let result = std::str::from_utf8(&connection.line)
                            .map_err(|_| super::IngestDenial::BadJson)
                            .and_then(|line| {
                                self.store.ingest_line(
                                    connection.peer,
                                    ticks,
                                    line.trim_end_matches('\r'),
                                    now_ns,
                                )
                            });
                        let reply = match result {
                            Ok(()) => {
                                self.accepted = self.accepted.saturating_add(1);
                                "ok\n".to_owned()
                            }
                            Err(denial) => {
                                self.loss_since_ns.get_or_insert(now_ns);
                                self.deny(denial_code(denial));
                                format!("denied {}\n", denial_code(denial))
                            }
                        };
                        connection.line.clear();
                        if connection.stream.write(reply.as_bytes()).ok() != Some(reply.len()) {
                            self.loss_since_ns.get_or_insert(now_ns);
                            break;
                        }
                    } else if connection.line.len() == MAX_SAMPLE_LINE_BYTES {
                        self.loss_since_ns.get_or_insert(now_ns);
                        self.deny("line_too_long");
                        break;
                    } else {
                        connection.line.push(byte[0]);
                    }
                    self.connection = Some(connection);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    self.connection = Some(connection);
                    break;
                }
                Err(_) => {
                    self.loss_since_ns.get_or_insert(now_ns);
                    self.deny("connection_error");
                    break;
                }
            }
        }
        self.observation("observing", now_ns, window_ns)
    }

    fn observation(&self, status: &str, now_ns: u64, window_ns: u64) -> KpiObservation {
        let metrics = std::iter::once((&self.contract.primary, "primary"))
            .chain(
                self.contract
                    .hard_guardrails
                    .iter()
                    .map(|spec| (spec, "hard_guardrail")),
            )
            .chain(
                self.contract
                    .advisory_guardrails
                    .iter()
                    .map(|spec| (spec, "advisory_guardrail")),
            )
            .map(|(spec, role)| KpiMetricObservation {
                metric: spec.id.clone(),
                unit: spec.unit.clone(),
                role: role.to_owned(),
                aggregate: self
                    .context_active
                    .then(|| {
                        self.store.window(
                            &spec.id,
                            now_ns
                                .saturating_sub(window_ns)
                                .max(self.context_started_ns),
                            now_ns,
                            now_ns,
                        )
                    })
                    .flatten()
                    .map(|mut aggregate| {
                        if self.loss_since_ns.is_some_and(|lost| lost <= now_ns)
                            || self
                                .connection
                                .as_ref()
                                .is_some_and(|connection| !connection.line.is_empty())
                        {
                            aggregate.complete = false;
                        }
                        aggregate
                    }),
            })
            .collect();
        KpiObservation {
            contract_digest: self.digest.clone(),
            source_session: self.source_session.clone(),
            binding: self.contract.binding,
            status: status.to_owned(),
            accepted_samples: self.accepted,
            denied_samples: self.denied,
            last_denial: self.last_denial.clone(),
            metrics,
            authorizes_mutation: false,
        }
    }
}

impl Drop for KpiCollector {
    fn drop(&mut self) {
        // Never remove a replacement socket installed by another writer.
        if std::fs::symlink_metadata(&self.socket_path)
            .is_ok_and(|meta| (meta.dev(), meta.ino()) == self.socket_identity)
        {
            let _ = std::fs::remove_file(&self.socket_path);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::DirBuilderExt;

    use super::*;

    struct Scratch(PathBuf);
    impl Scratch {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "stutter-kpi-collector-{name}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .unwrap();
            Self(path)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn binding(pid: u32) -> super::super::SourceBinding {
        let live = live_process(Path::new("/proc"), pid).unwrap();
        super::super::SourceBinding {
            uid: live.uid,
            pid,
            start_ticks: live.start_ticks,
        }
    }

    fn collector(dir: &Path, binding: super::super::SourceBinding) -> KpiCollector {
        KpiCollector::bind(super::super::audio_xrun(binding).contract, dir).unwrap()
    }

    fn now() -> u64 {
        crate::syscall::clock_gettime_ns(libc::CLOCK_MONOTONIC).unwrap()
    }

    #[test]
    fn kernel_bound_samples_are_observed_without_mutation_and_target_changes_erase_them() {
        let scratch = Scratch::new("good");
        let bound = binding(std::process::id());
        let mut collector = collector(&scratch.0, bound);
        let mut stream = UnixStream::connect(scratch.0.join("kpi.sock")).unwrap();
        let at = now();
        writeln!(stream, "{{\"metric\":\"deadline_miss_count\",\"value\":1,\"monotonic_ns\":{at},\"work_unit\":\"block-a\",\"population\":1000,\"kind\":\"batch_sum\"}}").unwrap();
        let target = Some((bound.pid, bound.start_ticks));
        let mut report = collector.poll(target, Path::new("/proc"), at, 1_000_000_000);
        for _ in 0..20 {
            if report.accepted_samples > 0 {
                break;
            }
            report = collector.poll(target, Path::new("/proc"), at, 1_000_000_000);
        }
        assert_eq!(report.accepted_samples, 1);
        assert!(!report.authorizes_mutation);
        assert_eq!(report.metrics[0].role, "primary");
        assert_eq!(report.metrics[1].role, "hard_guardrail");
        assert_eq!(
            report.metrics[0].aggregate.unwrap().semantics,
            super::super::Semantics::Direct
        );
        assert!(
            report.metrics[1].aggregate.is_none(),
            "a missing hard guardrail is not synthesized"
        );
        let lost = collector.poll(
            Some((bound.pid, bound.start_ticks + 1)),
            Path::new("/proc"),
            at,
            1_000_000_000,
        );
        assert_eq!(lost.status, "target_identity_mismatch");
        assert!(lost.metrics.iter().all(|metric| metric.aggregate.is_none()));
        let restored = collector.poll(target, Path::new("/proc"), at, 1_000_000_000);
        assert!(
            restored
                .metrics
                .iter()
                .all(|metric| metric.aggregate.is_none())
        );
        collector.boot_id = "wrong-boot".into();
        assert_eq!(
            collector.poll(target, Path::new("/proc"), at, 1).status,
            "target_identity_mismatch"
        );
    }

    #[test]
    fn a_partial_line_does_not_block_a_tick_and_an_oversize_line_is_bounded() {
        let scratch = Scratch::new("bounded");
        let bound = binding(std::process::id());
        let target = Some((bound.pid, bound.start_ticks));
        let mut collector = collector(&scratch.0, bound);
        let mut stream = UnixStream::connect(scratch.0.join("kpi.sock")).unwrap();
        stream.write_all(b"{").unwrap();
        let started = Instant::now();
        assert_eq!(
            collector
                .poll(target, Path::new("/proc"), now(), 1)
                .accepted_samples,
            0
        );
        assert!(started.elapsed() < Duration::from_millis(100));
        stream
            .write_all(&[b'x'; MAX_SAMPLE_LINE_BYTES + 1])
            .unwrap();
        for _ in 0..20 {
            collector.poll(target, Path::new("/proc"), now(), 1);
        }
        assert_eq!(collector.last_denial.as_deref(), Some("line_too_long"));
        assert!(collector.connection.is_none());
    }

    #[test]
    fn a_same_uid_but_different_process_cannot_report_and_socket_cleanup_respects_replacement() {
        let scratch = Scratch::new("wrong-peer");
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let bound = binding(child.id());
        let mut collector = collector(&scratch.0, bound);
        let _stream = UnixStream::connect(scratch.0.join("kpi.sock")).unwrap();
        let report = collector.poll(
            Some((bound.pid, bound.start_ticks)),
            Path::new("/proc"),
            now(),
            1,
        );
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(report.last_denial.as_deref(), Some("not_the_bound_process"));
        assert_eq!(report.accepted_samples, 0);
        let path = scratch.0.join("kpi.sock");
        std::fs::rename(&path, scratch.0.join("old.sock")).unwrap();
        std::fs::write(&path, b"external replacement").unwrap();
        drop(collector);
        assert_eq!(std::fs::read(&path).unwrap(), b"external replacement");
    }

    #[test]
    fn invalid_contracts_never_bind_and_normal_drop_removes_the_socket() {
        let scratch = Scratch::new("contract");
        let bound = binding(std::process::id());
        let mut contract = super::super::audio_xrun(bound).contract;
        contract.hard_guardrails.clear();
        assert!(KpiCollector::bind(contract, &scratch.0).is_err());
        assert!(!scratch.0.join("kpi.sock").exists());
        drop(collector(&scratch.0, bound));
        assert!(!scratch.0.join("kpi.sock").exists());
    }
}
