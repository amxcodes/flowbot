pub mod run_log;
pub mod isolated_agent;

use anyhow::{anyhow, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio_cron_scheduler::{Job, JobScheduler};
use uuid::Uuid;
use tokio::sync::mpsc;

/// Events sent by cron jobs when they fire
#[derive(Debug, Clone)]
pub enum CronEvent {
    SystemEvent { job_id: String, text: String },
    AgentTurn { job_id: String, message: String, model: Option<String>, thinking: Option<String>, timeout_seconds: Option<u64> },
}

/// Schedule types for cron jobs
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Schedule {
    /// One-shot execution at specific timestamp
    At { at_ms: u64 },
    /// Recurring interval execution
    Every { every_ms: u64, anchor_ms: Option<u64> },
    /// Cron expression-based scheduling
    Cron { expr: String, tz: Option<String> },
}

/// Payload types for cron jobs
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Payload {
    /// Inject text as system event into session
    SystemEvent { text: String },
    /// Run agent with message (isolated sessions only)
    AgentTurn {
        message: String,
        model: Option<String>,
        thinking: Option<String>,
        timeout_seconds: Option<u64>,
    },
}

/// Session target for cron job execution
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionTarget {
    Main,
    Isolated,
}

/// Wake mode for cron jobs
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WakeMode {
    NextHeartbeat,
    Now,
}

impl Default for WakeMode {
    fn default() -> Self {
        Self::NextHeartbeat
    }
}

/// Post-to-main mode for isolated jobs
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PostToMainMode {
    Summary,
    Full,
}

impl Default for PostToMainMode {
    fn default() -> Self {
        Self::Summary
    }
}

/// Isolation configuration for cron jobs
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CronIsolation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_to_main_prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_to_main_mode: Option<PostToMainMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_to_main_max_chars: Option<usize>,
}

/// Cron job definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    pub id: String,
    pub name: Option<String>,
    pub schedule: Schedule,
    pub payload: Payload,
    pub session_target: SessionTarget,
    pub enabled: bool,
    pub created_at: u64,
    #[serde(default)]
    pub wake_mode: WakeMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isolation: Option<CronIsolation>,
    #[serde(default)]
    pub delete_after_run: bool,
}

impl CronJob {
    pub fn new(name: Option<String>, schedule: Schedule, payload: Payload, session_target: SessionTarget) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        Self {
            id: Uuid::new_v4().to_string(),
            name,
            schedule,
            payload,
            session_target,
            enabled: true,
            created_at: now,
            wake_mode: WakeMode::default(),
            isolation: None,
            delete_after_run: false,
        }
    }
}

/// Cron scheduler manager
pub struct CronScheduler {
    db_path: PathBuf,
    scheduler: JobScheduler,
    event_tx: mpsc::Sender<CronEvent>,
}

impl CronScheduler {
    pub async fn new(db_path: PathBuf, event_tx: mpsc::Sender<CronEvent>) -> Result<Self> {
        let scheduler = JobScheduler::new().await?;
        Ok(Self { db_path, scheduler, event_tx })
    }

    /// Start the scheduler
    pub async fn start(&self) -> Result<()> {
        self.load_jobs().await?;
        self.scheduler.start().await?;
        Ok(())
    }

    /// Load existing jobs from database into runtime
    async fn load_jobs(&self) -> Result<()> {
        let jobs = self.list_jobs(false)?;
        println!("📂 Loading {} enabled cron jobs from DB", jobs.len());
        for job in jobs {
            self.register_job_runtime(job).await?;
        }
        Ok(())
    }

    /// Register a job in the tokio-cron-scheduler runtime
    async fn register_job_runtime(&self, job: CronJob) -> Result<()> {
        let job_id = job.id.clone();
        let job_name = job.name.clone().unwrap_or_else(|| "unnamed".to_string());
        println!("🚀 Registering cron job: {} ({})", job_id, job_name);
        
        let event_tx = self.event_tx.clone();
        let payload = job.payload.clone();
        
        match &job.schedule {
            Schedule::Cron { expr, .. } => {
                // Clone data for async closure
                let event_tx_clone = self.event_tx.clone();
                let job_id_clone = job_id.clone();
                let payload_clone = job.payload.clone();
                let db_path_clone = self.db_path.clone();
                let cron_expr = expr.clone();
                
                let job_handle = Job::new_async(cron_expr.as_str(), move |_uuid, _l| {
                    let tx = event_tx_clone.clone();
                    let id = job_id_clone.clone();
                    let payload = payload_clone.clone();
                    let db_path = db_path_clone.clone();
                    
                    Box::pin(async move {
                        let start_time = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_millis() as u64;
                        
                        let event = match payload {
                            Payload::SystemEvent { text } => {
                                CronEvent::SystemEvent { job_id: id.clone(), text }
                            }
                            Payload::AgentTurn { message, model, thinking, timeout_seconds } => {
                                CronEvent::AgentTurn { 
                                    job_id: id.clone(), 
                                    message, 
                                    model, 
                                    thinking, 
                                    timeout_seconds 
                                }
                            }
                        };
                        
                        // Send event
                        let result = tx.send(event).await;
                        
                        // Record execution to run log
                        let end_time = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_millis() as u64;
                        
                        let duration_ms = end_time - start_time;
                        let (status, error) = match &result {
                            Ok(_) => ("ok", None),
                            Err(e) => ("error", Some(e.to_string())),
                        };
                        
                        let log_path = run_log::resolve_run_log_path(&db_path, &id);
                        let entry = run_log::CronRunEntry {
                            ts: end_time,
                            job_id: id.clone(),
                            action: "finished".to_string(),
                            status: Some(status.to_string()),
                            error,
                            summary: None,
                            run_at_ms: Some(start_time),
                            duration_ms: Some(duration_ms),
                            next_run_at_ms: None,
                        };
                        
                        // Best effort logging - don't fail the job if logging fails
                        let _ = run_log::append_run_log(&log_path, &entry, 2_000_000, 2000);
                    })
                })?;
                
                self.scheduler.add(job_handle).await?;
            },
            Schedule::Every { every_ms, .. } => {
                 let job_id = job_id.clone();
                 let event_tx = event_tx.clone();
                 let payload = payload.clone();
                 
                 let tokio_job = Job::new_repeated_async(
                     std::time::Duration::from_millis(*every_ms), 
                     move |_uuid, _lock| {
                        let job_id = job_id.clone();
                        let event_tx = event_tx.clone();
                        let payload = payload.clone();
                        
                        Box::pin(async move {
                            let event = match payload {
                                Payload::SystemEvent { text } => CronEvent::SystemEvent { 
                                    job_id: job_id.clone(), 
                                    text 
                                },
                                Payload::AgentTurn { message, model, thinking, timeout_seconds } => {
                                    CronEvent::AgentTurn { 
                                        job_id: job_id.clone(), 
                                        message, 
                                        model, 
                                        thinking, 
                                        timeout_seconds 
                                    }
                                }
                            };
                            
                            if let Err(e) = event_tx.send(event).await {
                                eprintln!("Failed to send cron event for job {}: {}", job_id, e);
                            }
                        })
                 })?;
                 self.scheduler.add(tokio_job).await?;
            },
            Schedule::At { at_ms } => {
                let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
                if *at_ms > now {
                    let duration = std::time::Duration::from_millis(at_ms - now);
                    let job_id = job_id.clone();
                    let event_tx = event_tx.clone();
                    let payload = payload.clone();
                    
                    let tokio_job = Job::new_one_shot_async(duration, move |_uuid, _lock| {
                         let job_id = job_id.clone();
                         let event_tx = event_tx.clone();
                         let payload = payload.clone();
                         
                         Box::pin(async move {
                             let event = match payload {
                                 Payload::SystemEvent { text } => CronEvent::SystemEvent { 
                                     job_id: job_id.clone(), 
                                     text 
                                 },
                                 Payload::AgentTurn { message, model, thinking, timeout_seconds } => {
                                     CronEvent::AgentTurn { 
                                         job_id: job_id.clone(), 
                                         message, 
                                         model, 
                                         thinking, 
                                         timeout_seconds 
                                     }
                                 }
                             };
                             
                             if let Err(e) = event_tx.send(event).await {
                                 eprintln!("Failed to send cron event for job {}: {}", job_id, e);
                             }
                         })
                    })?;
                    self.scheduler.add(tokio_job).await?;
                } else {
                     eprintln!("Job {} scheduled in the past, skipping.", job_id);
                }
            }
        }
        Ok(())
    }

    /// Add a new cron job
    pub async fn add_job(&self, job: CronJob) -> Result<String> {
        // Validate constraints
        match (&job.session_target, &job.payload) {
            (SessionTarget::Main, Payload::SystemEvent { .. }) => {}, // Valid
            (SessionTarget::Isolated, Payload::AgentTurn { .. }) => {}, // Valid
            (SessionTarget::Main, Payload::AgentTurn { .. }) => {
                return Err(anyhow!("sessionTarget='main' requires payload.kind='systemEvent'"));
            }
            (SessionTarget::Isolated, Payload::SystemEvent { .. }) => {
                return Err(anyhow!("sessionTarget='isolated' requires payload.kind='agentTurn'"));
            }
        }

        // Save to database
        let conn = Connection::open(&self.db_path)?;
        conn.execute(
            "INSERT INTO cron_jobs (id, name, schedule_kind, schedule_data, payload_kind, payload_data, session_target, enabled, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                job.id,
                job.name,
                serde_json::to_string(&job.schedule)?,
                serde_json::to_string(&job.schedule)?,
                match &job.payload {
                    Payload::SystemEvent { .. } => "systemEvent",
                    Payload::AgentTurn { .. } => "agentTurn",
                },
                serde_json::to_string(&job.payload)?,
                match job.session_target {
                    SessionTarget::Main => "main",
                    SessionTarget::Isolated => "isolated",
                },
                job.enabled,
                job.created_at,
            ],
        )?;

        // Register with tokio-cron-scheduler
        self.register_job_runtime(job.clone()).await?;

        Ok(job.id)
    }

    /// List all cron jobs
    pub fn list_jobs(&self, include_disabled: bool) -> Result<Vec<CronJob>> {
        let conn = Connection::open(&self.db_path)?;
        
        let query = if include_disabled {
            "SELECT id, name, schedule_kind, schedule_data, payload_kind, payload_data, session_target, enabled, created_at FROM cron_jobs"
        } else {
            "SELECT id, name, schedule_kind, schedule_data, payload_kind, payload_data, session_target, enabled, created_at FROM cron_jobs WHERE enabled = 1"
        };

        let mut stmt = conn.prepare(query)?;
        let job_iter = stmt.query_map([], |row| {
            let schedule_data: String = row.get(3)?;
            let payload_data: String = row.get(5)?;
            let session_target_str: String = row.get(6)?;

            Ok(CronJob {
                id: row.get(0)?,
                name: row.get(1)?,
                schedule: serde_json::from_str(&schedule_data).unwrap(),
                payload: serde_json::from_str(&payload_data).unwrap(),
                session_target: match session_target_str.as_str() {
                    "main" => SessionTarget::Main,
                    _ => SessionTarget::Isolated,
                },
                enabled: row.get(7)?,
                created_at: row.get(8)?,
                wake_mode: WakeMode::default(),
                isolation: None,
                delete_after_run: false,
            })
        })?;

        let mut jobs = Vec::new();
        for job in job_iter {
            jobs.push(job?);
        }

        Ok(jobs)
    }

    /// Remove a cron job
    pub fn remove_job(&self, job_id: &str) -> Result<()> {
        let conn = Connection::open(&self.db_path)?;
        let affected = conn.execute(
            "DELETE FROM cron_jobs WHERE id = ?1",
            params![job_id],
        )?;

        if affected == 0 {
            return Err(anyhow!("Job not found: {}", job_id));
        }

        Ok(())
    }

    /// Get scheduler status
    pub async fn status(&self) -> Result<serde_json::Value> {
        let jobs = self.list_jobs(true)?;
        Ok(serde_json::json!({
            "running": true,
            "total_jobs": jobs.len(),
            "enabled_jobs": jobs.iter().filter(|j| j.enabled).count(),
        }))
    }
}
