//! Client side of the protocol, used by the campaign controller (normal user domain).
use std::{
    os::unix::net::UnixStream,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    model::{Envelope, Request, Response, SCHEMA_VERSION},
    protocol::{read_frame, write_frame},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

/// The environment variable that points the campaign at a running runner.
pub const SOCKET_ENV: &str = "STUTTER_VALIDATION_RUNNER";

pub struct Client<'a> {
    pub socket: &'a Path,
    pub campaign_id: &'a str,
    pub manifest_sha256: &'a str,
}

fn request_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!(
        "req-{nanos}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

impl Client<'_> {
    pub fn send(&self, task_id: &str, body: Request) -> std::io::Result<Response> {
        let envelope = Envelope {
            schema_version: SCHEMA_VERSION,
            campaign_id: self.campaign_id.to_owned(),
            task_id: task_id.to_owned(),
            request_id: request_id(),
            manifest_sha256: self.manifest_sha256.to_owned(),
            body,
        };
        let mut stream = UnixStream::connect(self.socket)?;
        write_frame(&mut stream, &envelope)?;
        read_frame(&mut stream)
    }
}
