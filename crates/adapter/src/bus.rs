//! The agent-bus client (`ARCHITECTURE` §4, `contract/adapter-v1.json`).
//!
//! The hub owns the adapter's **process**: it spawns it, speaks JSON-RPC 2.0
//! over its stdin/stdout (one LF-terminated UTF-8 line per message), correlates
//! replies by id, and receives notifications (the source of the SSE events).
//! No adapter code runs inside the hub's process.
//!
//! The connection is **split** so requests and notifications never contend:
//! [`RequestHandle`] writes requests and awaits replies (cheap to clone), and
//! [`Notifications`] drains the adapter's events. Neither blocks the other.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, thiserror::Error)]
pub enum BusError {
    #[error("spawn: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("the adapter closed its stdout")]
    Closed,
    #[error("rpc error {code}: {message}")]
    Rpc { code: i64, message: String, data: Value },
    #[error("invalid message: {0}")]
    Protocol(String),
}

/// A notification from the adapter (an event the hub turns into an SSE frame).
#[derive(Debug, Clone)]
pub struct Notification {
    pub method: String,
    pub params: Value,
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, BusError>>>>>;

/// The write half: send a request, await its reply. Cloneable.
#[derive(Clone)]
pub struct RequestHandle {
    stdin: Arc<tokio::sync::Mutex<ChildStdin>>,
    next_id: Arc<AtomicU64>,
    pending: Pending,
    /// False once the adapter's stdout closes (the process exited). A cached
    /// handle that is dead must not be reused: a one-shot capability adapter exits
    /// after it answers.
    alive: Arc<AtomicBool>,
}

impl RequestHandle {
    /// Whether the adapter process is still alive (its stdout is open).
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    pub async fn request(&self, method: &str, params: Value) -> Result<Value, BusError> {
        let rx = self.send(method, params).await?;
        rx.await.map_err(|_| BusError::Closed)?
    }

    /// WRITE the request frame and return the receiver for its reply, WITHOUT
    /// awaiting it. A caller that must serialize DELIVERY (a turn's prompt vs a
    /// cancel's abort) can hold a lock across the write and then release it before
    /// the long wait for the answer (TASK-048 F4).
    pub async fn send(
        &self,
        method: &str,
        params: Value,
    ) -> Result<oneshot::Receiver<Result<Value, BusError>>, BusError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().expect("pending").insert(id, tx);

        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let mut line = serde_json::to_string(&msg).expect("serialize request");
        line.push('\n');
        {
            let mut stdin = self.stdin.lock().await;
            stdin.write_all(line.as_bytes()).await?;
            stdin.flush().await?;
        }
        Ok(rx)
    }
}

/// The read half: the adapter's notifications.
pub struct Notifications {
    rx: mpsc::UnboundedReceiver<Notification>,
}

impl Notifications {
    pub async fn recv(&mut self) -> Option<Notification> {
        self.rx.recv().await
    }

    /// An empty receiver (used to move the real one out without an Option).
    pub fn closed() -> Self {
        let (_tx, rx) = mpsc::unbounded_channel();
        Notifications { rx }
    }
}

/// A running adapter process.
pub struct AgentBus {
    child: Child,
    pub requests: RequestHandle,
    pub notifications: Notifications,
    _reader: tokio::task::JoinHandle<()>,
}

impl AgentBus {
    /// Spawn an adapter: `command` is its argv, `cwd` its directory, and `env`
    /// extra variables (a connection's token reaches it here as `envName`, never
    /// in a payload).
    pub fn spawn(
        command: &[String],
        cwd: &std::path::Path,
        env: &[(String, String)],
    ) -> Result<Self, BusError> {
        let (program, args) = command
            .split_first()
            .ok_or_else(|| BusError::Protocol("the manifest declared no command".into()))?;
        let mut cmd = Command::new(program);
        cmd.args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().ok_or(BusError::Closed)?;
        let stdout = child.stdout.take().ok_or(BusError::Closed)?;

        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let (tx, rx) = mpsc::unbounded_channel();
        let alive = Arc::new(AtomicBool::new(true));

        let reader_pending = pending.clone();
        let reader_alive = alive.clone();
        let reader = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            const EOF_RETRIES: u32 = 20;
            let mut eof_streak: u32 = 0;
            loop {
                // Distinguish a real END OF STREAM (the adapter is gone) from a
                // transient read error: `while let Ok(Some(..))` treated an IO error
                // as "closed", which detached a LIVE adapter and failed a delivered
                // abort (TASK-048 rework). Only `Ok(None)` (EOF) or a repeated error
                // after the child has exited ends the read.
                let line = match lines.next_line().await {
                    Ok(Some(line)) => {
                        eof_streak = 0;
                        line
                    }
                    Ok(None) => {
                        // END OF STREAM. Confirmed by re-reading: a transient EOF (a
                        // pipe read racing the adapter's own flush) must NOT be
                        // mistaken for "the adapter exited", or a LIVE adapter gets
                        // detached and a delivered abort fails (TASK-048 rework).
                        // Only a SUSTAINED EOF means the adapter is gone.
                        eof_streak += 1;
                        if eof_streak <= EOF_RETRIES {
                            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                            continue;
                        }
                        break;
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "adapter read error; retrying");
                        continue;
                    }
                };
                if line.trim().is_empty() {
                    continue;
                }
                let msg: Value = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!(error = %e, "adapter sent a non-JSON line");
                        continue;
                    }
                };
                if msg.get("method").is_some() {
                    let method = msg["method"].as_str().unwrap_or_default().to_string();
                    let params = msg.get("params").cloned().unwrap_or(Value::Null);
                    let _ = tx.send(Notification { method, params });
                    continue;
                }
                if let Some(id) = msg.get("id").and_then(Value::as_u64) {
                    let sender = reader_pending.lock().expect("pending").remove(&id);
                    if let Some(sender) = sender {
                        if let Some(err) = msg.get("error") {
                            let code = err.get("code").and_then(Value::as_i64).unwrap_or(0);
                            let message = err
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("adapter error")
                                .to_string();
                            // Keep the adapter's typed `data` (its contract code), so
                            // the identity is not lost (TASK-048 F5).
                            let data = err.get("data").cloned().unwrap_or(Value::Null);
                            let _ = sender.send(Err(BusError::Rpc { code, message, data }));
                        } else {
                            let _ = sender.send(Ok(msg.get("result").cloned().unwrap_or(Value::Null)));
                        }
                    }
                }
            }
            // The adapter closed its stdout: it is gone. Mark it dead so a cached
            // handle is not reused.
            tracing::warn!("adapter stdout closed (reader ending)");
            reader_alive.store(false, Ordering::Relaxed);
            let mut pending = reader_pending.lock().expect("pending");
            for (_, sender) in pending.drain() {
                let _ = sender.send(Err(BusError::Closed));
            }
        });

        Ok(AgentBus {
            child,
            requests: RequestHandle {
                stdin: Arc::new(tokio::sync::Mutex::new(stdin)),
                next_id: Arc::new(AtomicU64::new(1)),
                pending,
                alive,
            },
            notifications: Notifications { rx },
            _reader: reader,
        })
    }

    /// Terminate the adapter and **confirm** it exited. Takes `&mut self` so a
    /// failed stop does NOT consume the bus: the caller keeps the handle and can
    /// retry (N3). The adapter's own child (its harness) exits when the adapter
    /// does (the adapter is responsible for it, per the contract); the hub does
    /// not scan for processes by name.
    pub async fn shutdown(&mut self) -> Result<(), BusError> {
        // A child that has ALREADY exited is a CONFIRMED stop (TASK-048 S3). A
        // stdout EOF alone is NOT proof of exit, so we consult the child itself.
        if let Ok(Some(_status)) = self.child.try_wait() {
            return Ok(());
        }
        // Kill, then WAIT: the exit is only confirmed when `wait` returns. A kill
        // error when the child has just exited on its own is not a failure.
        if let Err(e) = self.child.kill().await {
            if self.child.try_wait().ok().flatten().is_none() {
                return Err(e.into());
            }
        }
        let _status = self.child.wait().await?;
        Ok(())
    }
}

#[cfg(test)]
mod alive_tests {
    use super::*;

    /// A child that exits marks its handle dead: a cached handle is not reused.
    /// This is what makes a one-shot capability adapter safe to call repeatedly.
    #[tokio::test]
    async fn a_dead_child_is_reported_dead() {
        // A shell/one-shot command that exits immediately. Use the platform shell.
        let (program, args): (&str, Vec<String>) = if cfg!(windows) {
            ("cmd", vec!["/C".into(), "echo {}".into()])
        } else {
            ("sh", vec!["-c".into(), "true".into()])
        };
        let bus = AgentBus::spawn(
            &[program.to_string(), args[0].clone(), args[1].clone()],
            std::path::Path::new("."),
            &[],
        )
        .expect("spawn");
        let handle = bus.requests.clone();
        assert!(handle.is_alive(), "alive right after spawn");
        // Wait for the child to exit and the reader to observe EOF.
        for _ in 0..100 {
            if !handle.is_alive() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(!handle.is_alive(), "a handle whose child exited reports dead");
    }
}
