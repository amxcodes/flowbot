use anyhow::{Result, anyhow};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

// Global Process Manager
static PROCESS_MANAGER: Lazy<Arc<ProcessManager>> =
    Lazy::new(|| Arc::new(ProcessManager::new()));

struct ProcessManager {
    // We store handles to managing tasks/channels, not just the Child itself
    // because we move the Child's streams to background tasks.
    processes: Mutex<HashMap<u32, ProcessHandle>>,
}

struct ProcessHandle {
    input_tx: tokio::sync::mpsc::Sender<String>,
    output_buffer: Arc<Mutex<Vec<String>>>, // Circular buffer
    kill_tx: tokio::sync::mpsc::Sender<()>,
    exited_at: Arc<Mutex<Option<Instant>>>,
}

#[derive(Serialize)]
struct ProcessSnapshot {
    pid: u32,
    buffered_lines: usize,
    last_line: Option<String>,
}

impl ProcessManager {
    const EXITED_RETAIN_FOR: Duration = Duration::from_secs(300);
    const MAX_TRACKED_PROCESSES: usize = 256;

    fn new() -> Self {
        Self {
            processes: Mutex::new(HashMap::new()),
        }
    }

    fn cleanup_stale_locked(processes: &mut HashMap<u32, ProcessHandle>) {
        let now = Instant::now();
        let mut expired = Vec::new();
        let mut removed = 0u64;

        for (pid, handle) in processes.iter() {
            if let Ok(guard) = handle.exited_at.lock()
                && let Some(exited_at) = *guard
                && now.saturating_duration_since(exited_at) >= Self::EXITED_RETAIN_FOR
            {
                expired.push(*pid);
            }
        }

        for pid in expired {
            if processes.remove(&pid).is_some() {
                removed = removed.saturating_add(1);
            }
        }

        if processes.len() <= Self::MAX_TRACKED_PROCESSES {
            return;
        }

        let mut exited: Vec<(u32, Instant)> = processes
            .iter()
            .filter_map(|(pid, handle)| {
                handle
                    .exited_at
                    .lock()
                    .ok()
                    .and_then(|g| g.map(|t| (*pid, t)))
            })
            .collect();
        exited.sort_by_key(|(_, t)| *t);

        let over = processes.len().saturating_sub(Self::MAX_TRACKED_PROCESSES);
        for (pid, _) in exited.into_iter().take(over) {
            if processes.remove(&pid).is_some() {
                removed = removed.saturating_add(1);
            }
        }

        if removed > 0 {
            crate::metrics::GLOBAL_METRICS
                .increment_counter("process_manager_stale_entries_removed_total", removed);
        }
    }

    pub async fn spawn(&self, cmd: String, args: Vec<String>) -> Result<u32> {
        if !super::commands::command_allowed(&cmd) {
            return Err(anyhow!(
                "Command '{}' is not in the allowed whitelist.",
                cmd
            ));
        }

        if super::commands::dangerous_command_detected(&cmd, &args)
            && std::env::var("NANOBOT_ALLOW_DANGEROUS_COMMANDS")
                .ok()
                .as_deref()
                != Some("1")
        {
            return Err(anyhow!(
                "Blocked dangerous command. Set NANOBOT_ALLOW_DANGEROUS_COMMANDS=1 to override explicitly."
            ));
        }

        let mut child = Command::new(&cmd)
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| anyhow!("Failed to spawn {}: {}", cmd, e))?;

        let pid = child.id().ok_or_else(|| anyhow!("Process has no PID"))?;

        // Setup IO
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("stdout pipe unavailable"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("stderr pipe unavailable"))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("stdin pipe unavailable"))?;

        let output_buffer = Arc::new(Mutex::new(Vec::new()));
        let buffer_clone = output_buffer.clone();
        let exited_at = Arc::new(Mutex::new(None));
        let exited_at_clone = exited_at.clone();

        // Input Channel
        let (input_tx, mut input_rx) = tokio::sync::mpsc::channel::<String>(100);

        // Kill Channel
        let (kill_tx, mut kill_rx) = tokio::sync::mpsc::channel::<()>(1);

        // Background Task: IO Multiplexer
        tokio::spawn(async move {
            let mut stdout_reader = BufReader::new(stdout).lines();
            let mut stderr_reader = BufReader::new(stderr).lines();

            loop {
                tokio::select! {
                    // Read Stdout
                    line = stdout_reader.next_line() => {
                        match line {
                            Ok(Some(t)) => Self::append_log(&buffer_clone, format!("[STDOUT] {}", t)),
                            _ => break, // EOF
                        }
                    }
                    // Read Stderr
                    line = stderr_reader.next_line() => {
                        match line {
                            Ok(Some(t)) => Self::append_log(&buffer_clone, format!("[STDERR] {}", t)),
                            _ => break,
                        }
                    }
                    // Write Stdin
                    input = input_rx.recv() => {
                        if let Some(text) = input {
                            if (stdin.write_all(text.as_bytes()).await).is_err() {
                                break;
                            }
                            if (stdin.write_all(b"\n").await).is_err() {
                                break;
                            }
                        }
                    }
                    // Kill Signal
                    _ = kill_rx.recv() => {
                         let _ = child.kill().await;
                         break;
                    }
                    // Child Exit
                    _ = child.wait() => {
                        break;
                    }
                }
            }
            Self::append_log(&buffer_clone, "[PROCESS EXITED]".to_string());
            if let Ok(mut guard) = exited_at_clone.lock() {
                *guard = Some(Instant::now());
            }
        });

        let handle = ProcessHandle {
            input_tx,
            output_buffer,
            kill_tx,
            exited_at,
        };

        let mut lock = self
            .processes
            .lock()
            .map_err(|e| anyhow!("Process manager lock poisoned: {}", e))?;
        Self::cleanup_stale_locked(&mut lock);
        lock.insert(pid, handle);
        Ok(pid)
    }

    fn append_log(buffer: &Arc<Mutex<Vec<String>>>, line: String) {
        let Ok(mut lock) = buffer.lock() else { return };
        if lock.len() >= 100 {
            // Max 100 lines history
            lock.remove(0);
        }
        lock.push(line);
    }

    pub fn read_output(&self, pid: u32) -> Result<String> {
        let lock = self
            .processes
            .lock()
            .map_err(|e| anyhow!("Process manager lock poisoned: {}", e))?;
        let mut lock = lock;
        Self::cleanup_stale_locked(&mut lock);
        if let Some(handle) = lock.get(&pid) {
            let buffer = handle
                .output_buffer
                .lock()
                .map_err(|e| anyhow!("Output buffer lock poisoned: {}", e))?;
            Ok(buffer.join("\n"))
        } else {
            Err(anyhow!("Process {} not found", pid))
        }
    }

    pub async fn write_input(&self, pid: u32, input: String) -> Result<()> {
        let tx = {
            let lock = self
                .processes
                .lock()
                .map_err(|e| anyhow!("Process manager lock poisoned: {}", e))?;
            let mut lock = lock;
            Self::cleanup_stale_locked(&mut lock);
            if let Some(handle) = lock.get(&pid) {
                handle.input_tx.clone()
            } else {
                return Err(anyhow!("Process {} not found", pid));
            }
        };
        tx.send(input)
            .await
            .map_err(|_| anyhow!("Failed to send input"))
    }

    pub async fn kill(&self, pid: u32) -> Result<()> {
        let tx = {
            let mut lock = self
                .processes
                .lock()
                .map_err(|e| anyhow!("Process manager lock poisoned: {}", e))?;
            Self::cleanup_stale_locked(&mut lock);
            if let Some(handle) = lock.remove(&pid) {
                handle.kill_tx
            } else {
                return Err(anyhow!("Process {} not found", pid));
            }
        };
        let _ = tx.send(()).await;
        Ok(())
    }

    fn list(&self) -> Result<Vec<ProcessSnapshot>> {
        let lock = self
            .processes
            .lock()
            .map_err(|e| anyhow!("Process manager lock poisoned: {}", e))?;

        let mut lock = lock;
        Self::cleanup_stale_locked(&mut lock);

        let mut out = Vec::with_capacity(lock.len());
        for (pid, handle) in lock.iter() {
            let (buffered_lines, last_line) = match handle.output_buffer.lock() {
                Ok(buf) => (buf.len(), buf.last().cloned()),
                Err(_) => (0, None),
            };
            out.push(ProcessSnapshot {
                pid: *pid,
                buffered_lines,
                last_line,
            });
        }
        Ok(out)
    }
}

// Tool Arguments
#[derive(Serialize, Deserialize)]
pub(super) struct SpawnArgs {
    pub command: String,
    pub args: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct PidArgs {
    pub pid: u32,
}

#[derive(Serialize, Deserialize)]
pub(super) struct WriteInputArgs {
    pub pid: u32,
    pub input: String,
}

// Tool Functions
pub(super) async fn spawn_process(_token: &super::ExecutorToken, args: SpawnArgs) -> Result<String> {
    let args_vec = args.args.unwrap_or_default();
    let pid = PROCESS_MANAGER
        .spawn(args.command.clone(), args_vec)
        .await?;
    Ok(format!("Started process {} (PID: {})", args.command, pid))
}

pub(super) async fn read_process_output(_token: &super::ExecutorToken, args: PidArgs) -> Result<String> {
    PROCESS_MANAGER.read_output(args.pid)
}

pub(super) async fn write_process_input(_token: &super::ExecutorToken, args: WriteInputArgs) -> Result<String> {
    PROCESS_MANAGER.write_input(args.pid, args.input).await?;
    Ok(format!("Sent input to PID {}", args.pid))
}

pub(super) async fn terminate_process(_token: &super::ExecutorToken, args: PidArgs) -> Result<String> {
    PROCESS_MANAGER.kill(args.pid).await?;
    Ok(format!("Terminated PID {}", args.pid))
}

pub(super) async fn list_processes(_token: &super::ExecutorToken) -> Result<String> {
    let processes = PROCESS_MANAGER.list()?;
    Ok(serde_json::to_string(&serde_json::json!({
        "count": processes.len(),
        "processes": processes,
    }))?)
}
