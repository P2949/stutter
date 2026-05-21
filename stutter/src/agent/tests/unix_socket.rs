//! Agent Unix socket server limit tests.

use std::{
    path::{Path as StdPath, PathBuf},
    sync::Arc,
    time::Duration,
};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
};

use super::{support::*, *};

#[tokio::test]
async fn unix_socket_connection_cap_rejects_extra_idle_clients() {
    let (socket_path, server) =
        start_unix_socket_server("connection_cap", 2, Duration::from_secs(5)).await;

    let first = UnixStream::connect(&socket_path).await.unwrap();
    let second = UnixStream::connect(&socket_path).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    let rejected = UnixStream::connect(&socket_path).await.unwrap();
    assert_stream_closes_without_response(rejected).await;

    drop(first);
    drop(second);
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn unix_socket_idle_connection_is_closed_after_timeout() {
    let (socket_path, server) =
        start_unix_socket_server("idle_timeout", 1, Duration::from_millis(50)).await;
    let idle = UnixStream::connect(&socket_path).await.unwrap();

    assert_idle_stream_closes(idle).await;

    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn unix_socket_active_request_still_works_with_timeout_enabled() {
    let (socket_path, server) =
        start_unix_socket_server("active_request", 1, Duration::from_secs(2)).await;

    assert_version_request_succeeds(&socket_path).await;

    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn unix_socket_timed_out_idle_connection_releases_permit() {
    let (socket_path, server) =
        start_unix_socket_server("timeout_releases_permit", 1, Duration::from_millis(50)).await;
    let idle = UnixStream::connect(&socket_path).await.unwrap();
    assert_idle_stream_closes(idle).await;

    assert_version_request_succeeds(&socket_path).await;

    server.abort();
    let _ = server.await;
}

async fn start_unix_socket_server(
    name: &str,
    max_connections: usize,
    connection_timeout: Duration,
) -> (PathBuf, tokio::task::JoinHandle<anyhow::Result<()>>) {
    let dir = agent_unix_socket_temp_dir(name);
    let socket_path = dir.join("agent.sock");
    let mut state_value = test_agent_state("127.0.0.1:0".parse().unwrap(), None);
    state_value.unix_socket = Some(socket_path.clone());
    let state = Arc::new(state_value);
    let app =
        crate::agent::routes::build_agent_router(state, Arc::new(AgentRateLimiter::default()));

    let server = tokio::spawn(crate::agent::server::serve_unix_socket_with_limits(
        socket_path.clone(),
        app,
        max_connections,
        connection_timeout,
    ));
    wait_for_unix_socket_path(&socket_path).await;
    (socket_path, server)
}

async fn assert_stream_closes_without_response(mut stream: UnixStream) {
    let request = b"GET /version HTTP/1.1\r\nHost: localhost\r\n\r\n";
    match stream.write_all(request).await {
        Ok(()) => {}
        Err(err)
            if matches!(
                err.kind(),
                std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
            ) =>
        {
            return;
        }
        Err(err) => panic!("unexpected write error from rejected unix stream: {err}"),
    }

    let mut byte = [0_u8; 1];
    let read = tokio::time::timeout(Duration::from_secs(1), stream.read(&mut byte))
        .await
        .expect("rejected unix stream should close promptly");
    match read {
        Ok(0) => {}
        Ok(n) => panic!("rejected unix stream returned {n} response bytes"),
        Err(err)
            if matches!(
                err.kind(),
                std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
            ) => {}
        Err(err) => panic!("unexpected read error from rejected unix stream: {err}"),
    }
}

async fn assert_idle_stream_closes(mut stream: UnixStream) {
    let mut byte = [0_u8; 1];
    let read = tokio::time::timeout(Duration::from_secs(1), stream.read(&mut byte))
        .await
        .expect("idle unix stream should close after timeout");
    match read {
        Ok(0) => {}
        Ok(n) => panic!("timed-out idle unix stream returned {n} response bytes"),
        Err(err)
            if matches!(
                err.kind(),
                std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
            ) => {}
        Err(err) => panic!("unexpected read error from idle unix stream: {err}"),
    }
}

async fn assert_version_request_succeeds(path: &StdPath) {
    let mut stream = UnixStream::connect(path).await.unwrap();
    stream
        .write_all(b"GET /version HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();

    let mut buffer = [0_u8; 4096];
    let read = tokio::time::timeout(Duration::from_secs(1), stream.read(&mut buffer))
        .await
        .expect("version request should receive a response")
        .unwrap();
    let response = String::from_utf8_lossy(&buffer[..read]);
    assert!(response.contains("200 OK"), "response={response}");
}

async fn wait_for_unix_socket_path(path: &StdPath) {
    for _ in 0..100 {
        if path.exists() {
            return;
        }
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(1)).await;
    }

    panic!("unix socket path was not created: {}", path.display());
}

fn agent_unix_socket_temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "stutter-agent-unix-socket-test-{name}-{}",
        crate::audit::unix_nanos_now()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
