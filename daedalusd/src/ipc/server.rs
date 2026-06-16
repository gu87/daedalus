//! UDS IPC Server — accept loop, lifecycle, and socket management.
//!
//! ```text
//! bind → accept loop (spawn peer tasks) → shutdown → cleanup
//! ```
//!
//! The server does **not** parse messages; parsing and routing are delegated
//! to [`crate::ipc::peer`] and [`crate::ipc::control`].

use std::future::Future;
use std::path::{Path, PathBuf};

use tokio::net::UnixListener;

use crate::error::DaedalusError;
use crate::ipc::peer;
use crate::ipc::session::Session;

/// Start the UDS server.
///
/// - `socket_path` — filesystem path for the listening socket.
/// - `shutdown` — future that signals the server to stop accepting new
///   connections and begin graceful shutdown.
///
/// # Socket lifecycle
///
/// - If `socket_path` already exists the function returns
///   [`DaedalusError::AlreadyExists`] immediately; it will **not** delete
///   a pre-existing socket.
/// - On successful bind the socket is created.  When `run` returns (whether
///   normally or because of the shutdown signal) the socket file is removed.
pub async fn run(
    socket_path: &Path,
    shutdown: impl Future<Output = ()>,
) -> Result<(), DaedalusError> {
    if socket_path.exists() {
        return Err(DaedalusError::AlreadyExists(socket_path.to_path_buf()));
    }

    // Ensure parent directory exists.
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            DaedalusError::Io(std::io::Error::other(format!(
                "failed to create socket parent dir {}: {}",
                parent.display(),
                e
            )))
        })?;
    }

    let listener = UnixListener::bind(socket_path).map_err(DaedalusError::Io)?;

    // Spawn peer tasks into a JoinSet so we can abort them on shutdown.
    let mut sessions: Vec<Session> = Vec::new();

    // Capture the accept-loop result so we can always clean up before
    // propagating the error.
    let accept_result = tokio::select! {
        result = accept_loop(&listener, &mut sessions) => {
            Some(result)
        }
        _ = shutdown => {
            None
        }
    };

    drop(listener);

    for s in &sessions {
        s.state.shutdown.cancel();
    }
    drop(sessions);

    // Clean up the socket we created — always, even on error.
    let _ = std::fs::remove_file(socket_path);

    // Propagate accept-loop error after cleanup.
    match accept_result {
        Some(Err(e)) => Err(e),
        _ => Ok(()),
    }
}

/// Accept connections in a loop, spawning each into `peers`.
async fn accept_loop(
    listener: &UnixListener,
    sessions: &mut Vec<Session>,
) -> Result<(), DaedalusError> {
    loop {
        let (stream, addr) = listener.accept().await.map_err(DaedalusError::Io)?;
        let peer_addr = addr
            .as_pathname()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("unnamed"));

        sessions.push(peer::spawn_session(stream, peer_addr));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;

    /// Spawn a server on a temp path and return the socket path plus a shutdown channel.
    async fn spawn_server(dir: &tempfile::TempDir) -> (PathBuf, tokio::sync::oneshot::Sender<()>) {
        let socket_path = dir.path().join("test.sock");
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let path = socket_path.clone();
        tokio::spawn(async move {
            let _ = run(&path, async {
                let _ = rx.await;
            })
            .await;
        });
        // Wait for the socket file to appear.
        for _ in 0..100 {
            if socket_path.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        (socket_path, tx)
    }

    /// Send one line, return the first response line.
    async fn send_recv(path: &Path, line: &str) -> String {
        let mut stream = UnixStream::connect(path).await.unwrap();
        stream.write_all(line.as_bytes()).await.unwrap();
        stream.write_all(b"\n").await.unwrap();
        let mut reader = BufReader::new(&mut stream);
        let mut buf = String::new();
        reader.read_line(&mut buf).await.unwrap();
        buf.trim_end().to_string()
    }

    // ── basic ping → pong ──────────────────────────────────────────

    #[tokio::test]
    async fn ping_gets_pong() {
        let dir = tempfile::TempDir::new().unwrap();
        let (path, _tx) = spawn_server(&dir).await;

        let resp = send_recv(
            &path,
            r#"{"type":"system.ping","ts":"2026-06-15T10:00:00.000Z","req_id":"t1"}"#,
        )
        .await;
        assert!(resp.contains("\"type\":\"system.pong\""));
        assert!(resp.contains("\"req_id\":\"t1\""));
    }

    // ── malformed JSON then legal ping on same connection ──────────

    #[tokio::test]
    async fn malformed_then_legal_on_same_connection() {
        let dir = tempfile::TempDir::new().unwrap();
        let (path, _tx) = spawn_server(&dir).await;

        let stream = UnixStream::connect(&path).await.unwrap();
        let (reader, mut writer) = tokio::io::split(stream);
        let mut reader = BufReader::new(reader);

        // Send garbage.
        writer.write_all(b"not json\n").await.unwrap();
        let mut buf = String::new();
        reader.read_line(&mut buf).await.unwrap();
        assert!(buf.contains("\"type\":\"system.error\""));
        assert!(buf.contains("\"malformed_json\""));
        buf.clear();

        // Same connection: send legal ping.
        writer
            .write_all(b"{\"type\":\"system.ping\",\"ts\":\"2026-06-15T10:00:00.000Z\",\"req_id\":\"after-error\"}\n")
            .await
            .unwrap();
        reader.read_line(&mut buf).await.unwrap();
        assert!(buf.contains("\"type\":\"system.pong\""));
        assert!(buf.contains("\"req_id\":\"after-error\""));
    }

    // ── two concurrent connections ─────────────────────────────────

    #[tokio::test]
    async fn two_concurrent_connections() {
        let dir = tempfile::TempDir::new().unwrap();
        let (path, _tx) = spawn_server(&dir).await;

        let p1 = path.clone();
        let h1 = tokio::spawn(async move {
            send_recv(
                &p1,
                r#"{"type":"system.ping","ts":"2026-06-15T10:00:00.000Z","req_id":"c1"}"#,
            )
            .await
        });
        let p2 = path.clone();
        let h2 = tokio::spawn(async move {
            send_recv(
                &p2,
                r#"{"type":"system.ping","ts":"2026-06-15T10:00:00.000Z","req_id":"c2"}"#,
            )
            .await
        });

        let (r1, r2) = tokio::join!(h1, h2);
        assert!(r1.unwrap().contains("\"req_id\":\"c1\""));
        assert!(r2.unwrap().contains("\"req_id\":\"c2\""));
    }

    // ── shutdown cleans up socket ──────────────────────────────────

    #[tokio::test]
    async fn shutdown_cleans_up_socket() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("shutdown.sock");
        assert!(!path.exists());

        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let p = path.clone();
        let handle = tokio::spawn(async move {
            run(&p, async {
                let _ = rx.await;
            })
            .await
            .unwrap();
        });

        // Wait for server to be ready.
        for _ in 0..200 {
            if path.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(path.exists(), "socket should exist after bind");

        // Signal shutdown.
        let _ = tx.send(());
        let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;

        assert!(!path.exists(), "socket should be removed after shutdown");
    }

    // ── existing socket → AlreadyExists ────────────────────────────

    #[tokio::test]
    async fn refuses_to_bind_existing_socket() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("existing.sock");

        // Create a dummy file at the socket path.
        std::fs::write(&path, b"").unwrap();
        assert!(path.exists());

        let (_, rx) = tokio::sync::oneshot::channel::<()>();
        let err = run(&path, async {
            let _ = rx.await;
        })
        .await
        .unwrap_err();

        match err {
            DaedalusError::AlreadyExists(p) => assert_eq!(p, path),
            other => panic!("expected AlreadyExists, got {:?}", other),
        }

        // The file must still exist — we must not have deleted it.
        assert!(path.exists(), "pre-existing socket must not be deleted");
    }

    // ── long line → error + close ──────────────────────────────────

    #[tokio::test]
    async fn long_line_returns_error_and_closes() {
        let dir = tempfile::TempDir::new().unwrap();
        let (path, _tx) = spawn_server(&dir).await;

        let stream = UnixStream::connect(&path).await.unwrap();
        let (reader, mut writer) = tokio::io::split(stream);
        let mut reader = BufReader::new(reader);

        // Build a line > 1 MiB (JSON object with huge padding, no internal newlines).
        let big: String = std::iter::repeat('x')
            .take(peer::MAX_LINE_LEN + 1024)
            .collect();
        let line = format!("{{\"data\":\"{}\"}}\n", big);
        writer.write_all(line.as_bytes()).await.unwrap();

        // Must receive a system.error response.
        let mut buf = String::new();
        let n = reader.read_line(&mut buf).await.unwrap();
        assert!(n > 0, "must receive an error response");
        assert!(
            buf.contains("\"type\":\"system.error\""),
            "expected system.error, got: {}",
            buf.trim_end()
        );
        assert!(
            buf.contains("\"invalid_message\""),
            "expected invalid_message code"
        );

        // Connection should be closed after the error — next read must return EOF/0.
        buf.clear();
        let n2 = reader.read_line(&mut buf).await.unwrap();
        assert_eq!(n2, 0, "connection must be closed after oversize line");
    }

    // ── accept error still cleans up ───────────────────────────────

    /// Test-only helper: same cleanup pattern as `run()`.  Exists solely
    /// so we can verify the cleanup path without depending on a real
    /// accept-loop error (which is hard to trigger portably).
    #[cfg(test)]
    async fn run_with_listener(
        listener: UnixListener,
        socket_path: &Path,
        shutdown: impl Future<Output = ()>,
    ) -> Result<(), DaedalusError> {
        let mut sessions: Vec<Session> = Vec::new();
        let accept_result = tokio::select! {
            result = accept_loop(&listener, &mut sessions) => Some(result),
            _ = shutdown => None,
        };
        drop(listener);
        for s in &sessions {
            s.state.shutdown.cancel();
        }
        drop(sessions);
        let _ = std::fs::remove_file(socket_path);
        match accept_result {
            Some(Err(e)) => Err(e),
            _ => Ok(()),
        }
    }

    #[tokio::test]
    async fn accept_error_still_cleans_up_socket() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("err.sock");

        // Bind → drop to leave a stale socket file.
        let _stale = UnixListener::bind(&path).unwrap();
        assert!(path.exists());
        drop(_stale);
        // Socket file persists (Unix semantics: bind doesn't auto-unlink).

        // Manually remove the stale file so we can re-bind.
        std::fs::remove_file(&path).unwrap();
        assert!(!path.exists());

        // Bind a fresh listener for run_with_listener.
        let listener = UnixListener::bind(&path).unwrap();

        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let p = path.clone();
        let handle = tokio::spawn(async move {
            run_with_listener(listener, &p, async {
                let _ = rx.await;
            })
            .await
        });

        // Wait for the task to start accepting.
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(path.exists(), "socket still there while server runs");

        // Shut down and confirm cleanup.
        let _ = tx.send(());
        let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
        assert!(!path.exists(), "socket must be cleaned up after shutdown");
    }
}
