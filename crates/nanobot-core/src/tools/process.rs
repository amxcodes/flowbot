use anyhow::{Result, anyhow};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

// Global Process Manager
pub static PROCESS_MANAGER: Lazy<Arc<ProcessManager>> =
    Lazy::new(|| Arc::new(ProcessManager::new()));

pub struct ProcessManager {
    // We store handles to managing tasks/channels, not just the Child itself
    // because we move the Child's streams to background tasks.
    processes: Mutex<HashMap<u32, ProcessHandle>>,
}

struct ProcessHandle {
    input_tx: tokio::sync::mpsc::Sender<String>,
    output_buffer: Arc<Mutex<Vec<String>>>, // Circular buffer
    kill_tx: tokio::sync::mpsc::Sender<()>,
}

#[derive(Serialize)]
struct ProcessSnapshot {
    pid: u32,
    buffered_lines: usize,
    last_line: Option<String>,
}

impl ProcessManager {
    fn new() -> Self {
        Self {
            processes: Mutex::new(HashMap::new()),
        }
    }

    pub async fn spawn(&self, cmd: String, args: Vec<String>) -> Result<u32> {
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
        let stdout = child.stdout.take().expect("stdout should be piped");
        let stderr = child.stderr.take().expect("stderr should be piped");
        let mut stdin = child.stdin.take().expect("stdin should be piped");

        let output_buffer = Arc::new(Mutex::new(Vec::new()));
        let buffer_clone = output_buffer.clone();

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
        });

        let handle = ProcessHandle {
            input_tx,
            output_buffer,
            kill_tx,
        };

        self.processes.lock()
            .map_err(|e| anyhow!("Process manager lock poisoned: {}", e))?
            .insert(pid, handle);
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
        let lock = self.processes.lock().map_err(|e| anyhow!("Process manager lock poisoned: {}", e))?;
        if let Some(handle) = lock.get(&pid) {
            let buffer = handle.output_buffer.lock().map_err(|e| anyhow!("Output buffer lock poisoned: {}", e))?;
            Ok(buffer.join("\n"))
        } else {
            Err(anyhow!("Process {} not found", pid))
        }
    }

    pub async fn write_input(&self, pid: u32, input: String) -> Result<()> {
        let tx = {
            let lock = self.processes.lock().map_err(|e| anyhow!("Process manager lock poisoned: {}", e))?;
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
            let mut lock = self.processes.lock().map_err(|e| anyhow!("Process manager lock poisoned: {}", e))?;
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
pub struct SpawnArgs {
    pub command: String,
    pub args: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize)]
pub struct PidArgs {
    pub pid: u32,
}

#[derive(Serialize, Deserialize)]
pub struct WriteInputArgs {
    pub pid: u32,
    pub input: String,
}

// Tool Functions
pub async fn spawn_process(args: SpawnArgs) -> Result<String> {
    let args_vec = args.args.unwrap_or_default();
    let pid = PROCESS_MANAGER
        .spawn(args.command.clone(), args_vec)
        .await?;
    Ok(format!("Started process {} (PID: {})", args.command, pid))
}

pub async fn read_process_output(args: PidArgs) -> Result<String> {
    PROCESS_MANAGER.read_output(args.pid)
}

pub async fn write_process_input(args: WriteInputArgs) -> Result<String> {
    PROCESS_MANAGER.write_input(args.pid, args.input).await?;
    Ok(format!("Sent input to PID {}", args.pid))
}

pub async fn terminate_process(args: PidArgs) -> Result<String> {
    PROCESS_MANAGER.kill(args.pid).await?;
    Ok(format!("Terminated PID {}", args.pid))
}

pub async fn list_processes() -> Result<String> {
    let processes = PROCESS_MANAGER.list()?;
    Ok(serde_json::to_string(&serde_json::json!({
        "count": processes.len(),
        "processes": processes,
    }))?)
}
