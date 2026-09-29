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
use std::sync::atomic::{AtomicU64, Ordering};
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
    Rpc { code: i64, message: String },
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
}

impl RequestHandle {
    pub async fn request(&self, method: &str, params: Value) -> Result<Value, BusError> {
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
        rx.await.map_err(|_| BusError::Closed)?
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
}

/// A running adapter process.
pub struct AgentBus {
    child: Child,
    pub requests: RequestHandle,
    pub notifications: Notifications,
    reader: tokio::task::JoinHandle<()>,
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

        let reader_pending = pending.clone();
        let reader = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
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
                            let _ = sender.send(Err(BusError::Rpc { code, message }));
                        } else {
                            let _ = sender.send(Ok(msg.get("result").cloned().unwrap_or(Value::Null)));
                        }
                    }
                }
            }
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
            },
            notifications: Notifications { rx },
            reader,
        })
    }

    /// Terminate the adapter.
    pub async fn shutdown(mut self) {
        let _ = self.child.kill().await;
        let _ = self.reader.await;
    }
}
