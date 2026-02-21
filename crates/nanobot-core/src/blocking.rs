use anyhow::{Result, anyhow};
use once_cell::sync::Lazy;
use std::time::Duration;
use tokio::sync::Semaphore;

struct BlockingPool {
    name: &'static str,
    limit: usize,
    semaphore: Semaphore,
}

fn pool_limit(env_key: &str, default_limit: usize) -> usize {
    std::env::var(env_key)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(default_limit)
}

static FS_POOL: Lazy<BlockingPool> = Lazy::new(|| {
    let limit = pool_limit("NANOBOT_BLOCKING_FS_LIMIT", 16);
    BlockingPool {
        name: "fs",
        limit,
        semaphore: Semaphore::new(limit),
    }
});

static SQLITE_POOL: Lazy<BlockingPool> = Lazy::new(|| {
    let limit = pool_limit("NANOBOT_BLOCKING_SQLITE_LIMIT", 16);
    BlockingPool {
        name: "sqlite",
        limit,
        semaphore: Semaphore::new(limit),
    }
});

static PROCESS_POOL: Lazy<BlockingPool> = Lazy::new(|| {
    let limit = pool_limit("NANOBOT_BLOCKING_PROCESS_LIMIT", 8);
    BlockingPool {
        name: "process",
        limit,
        semaphore: Semaphore::new(limit),
    }
});

fn sanitize_metric_component(component: &str) -> String {
    component
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

async fn run_blocking_pool<T, F>(pool: &'static BlockingPool, op_name: &'static str, f: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    let wait_started = std::time::Instant::now();
    let permit = pool
        .semaphore
        .acquire()
        .await
        .map_err(|_| anyhow!("{} blocking semaphore closed", pool.name))?;

    crate::metrics::GLOBAL_METRICS.record_duration(
        &format!("blocking_semaphore_wait_seconds{{pool={}}}", pool.name),
        wait_started.elapsed(),
        true,
    );
    crate::metrics::GLOBAL_METRICS.set_gauge(
        &format!("blocking_tasks_inflight{{pool={}}}", pool.name),
        (pool
            .limit
            .saturating_sub(pool.semaphore.available_permits())) as f64,
    );

    let op_label = sanitize_metric_component(op_name);
    let started = std::time::Instant::now();
    let result = tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| anyhow!("blocking {} task join error: {}", pool.name, e))?;

    let success = result.is_ok();
    crate::metrics::GLOBAL_METRICS.record_duration(
        &format!(
            "blocking_operation_duration_seconds{{pool={},op={}}}",
            pool.name, op_label
        ),
        started.elapsed(),
        success,
    );

    drop(permit);
    crate::metrics::GLOBAL_METRICS.set_gauge(
        &format!("blocking_tasks_inflight{{pool={}}}", pool.name),
        (pool
            .limit
            .saturating_sub(pool.semaphore.available_permits())) as f64,
    );

    result
}

pub async fn fs<T, F>(op_name: &'static str, f: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    run_blocking_pool(&FS_POOL, op_name, f).await
}

pub async fn sqlite<T, F>(op_name: &'static str, f: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    run_blocking_pool(&SQLITE_POOL, op_name, f).await
}

pub async fn process_output(
    command: String,
    args: Vec<String>,
    timeout: Duration,
) -> Result<std::process::Output> {
    process_output_in_dir(command, args, timeout, None).await
}

pub async fn process_output_in_dir(
    command: String,
    args: Vec<String>,
    timeout: Duration,
    cwd: Option<std::path::PathBuf>,
) -> Result<std::process::Output> {
    let wait_started = std::time::Instant::now();
    let permit = PROCESS_POOL
        .semaphore
        .acquire()
        .await
        .map_err(|_| anyhow!("process blocking semaphore closed"))?;

    crate::metrics::GLOBAL_METRICS.record_duration(
        "blocking_semaphore_wait_seconds{pool=process}",
        wait_started.elapsed(),
        true,
    );
    crate::metrics::GLOBAL_METRICS.set_gauge(
        "blocking_tasks_inflight{pool=process}",
        (PROCESS_POOL
            .limit
            .saturating_sub(PROCESS_POOL.semaphore.available_permits())) as f64,
    );

    let started = std::time::Instant::now();
    let mut cmd = tokio::process::Command::new(&command);
    cmd.args(&args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    let child = cmd
        .spawn()
        .map_err(|e| anyhow!("failed to spawn process '{}': {}", command, e))?;

    let output = match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(o)) => Ok(o),
        Ok(Err(e)) => Err(anyhow!("process '{}' execution failed: {}", command, e)),
        Err(_) => Err(anyhow!("process '{}' timed out after {:?}", command, timeout)),
    };

    crate::metrics::GLOBAL_METRICS.record_duration(
        "blocking_operation_duration_seconds{pool=process,op=process_output}",
        started.elapsed(),
        output.is_ok(),
    );

    drop(permit);
    crate::metrics::GLOBAL_METRICS.set_gauge(
        "blocking_tasks_inflight{pool=process}",
        (PROCESS_POOL
            .limit
            .saturating_sub(PROCESS_POOL.semaphore.available_permits())) as f64,
    );

    output
}

pub async fn command_exists(command: &str, timeout: Duration) -> bool {
    process_output(command.to_string(), vec!["--version".to_string()], timeout)
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}
