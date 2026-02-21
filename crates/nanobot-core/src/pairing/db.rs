use anyhow::Result;
use once_cell::sync::Lazy;
use rusqlite::{Connection, params};
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;

static PAIRING_DB_BLOCKING_SEMAPHORE: Lazy<Semaphore> =
    Lazy::new(|| Semaphore::new(pairing_db_blocking_limit()));

fn pairing_db_blocking_limit() -> usize {
    std::env::var("NANOBOT_PAIRING_DB_BLOCKING_LIMIT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(16)
}

async fn run_db<T, F>(f: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(&Connection) -> Result<T> + Send + 'static,
{
    let wait_started = std::time::Instant::now();
    let permit = PAIRING_DB_BLOCKING_SEMAPHORE
        .acquire()
        .await
        .map_err(|_| anyhow::anyhow!("pairing db semaphore closed"))?;
    crate::metrics::GLOBAL_METRICS.record_duration(
        "blocking_semaphore_wait_seconds{pool=pairing_db}",
        wait_started.elapsed(),
        true,
    );
    crate::metrics::GLOBAL_METRICS.set_gauge(
        "blocking_tasks_inflight{pool=pairing_db}",
        (pairing_db_blocking_limit()
            .saturating_sub(PAIRING_DB_BLOCKING_SEMAPHORE.available_permits())) as f64,
    );

    let result = tokio::task::spawn_blocking(move || {
        let conn = DB_CONN
            .lock()
            .map_err(|e| anyhow::anyhow!("Database lock poisoned: {}", e))?;
        f(&conn)
    })
    .await
    .map_err(|e| anyhow::anyhow!("Pairing DB task join error: {}", e))?;

    drop(permit);
    crate::metrics::GLOBAL_METRICS.set_gauge(
        "blocking_tasks_inflight{pool=pairing_db}",
        (pairing_db_blocking_limit()
            .saturating_sub(PAIRING_DB_BLOCKING_SEMAPHORE.available_permits())) as f64,
    );

    result
}

static DB_CONN: Lazy<Arc<Mutex<Connection>>> = Lazy::new(|| {
    let db_path = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".nanobot")
        .join("pairing.db");
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // Legacy migration: flowbot.db -> ~/.nanobot/pairing.db
    let legacy_path = std::path::PathBuf::from("flowbot.db");
    if !db_path.exists() && legacy_path.exists() {
        let _ = std::fs::copy(&legacy_path, &db_path);
    }

    let conn = match Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                "Failed to open pairing database at {}: {}. Falling back to in-memory DB.",
                db_path.display(),
                e
            );
            Connection::open_in_memory()
                .expect("Failed to open fallback in-memory pairing database")
        }
    };
    Arc::new(Mutex::new(conn))
});

#[derive(Debug, Clone)]
pub struct PairingRequest {
    pub channel: String,
    pub user_id: String,
    pub username: Option<String>,
    pub code: String,
    pub created_at: i64,
    pub expires_at: i64,
}

/// Initialize the database with pairing tables
pub async fn init_database() -> Result<()> {
    run_db(|conn| {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS pairing_requests (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                channel TEXT NOT NULL,
                user_id TEXT NOT NULL,
                username TEXT,
                code TEXT NOT NULL UNIQUE,
                created_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_pairing_channel_code 
             ON pairing_requests(channel, code)",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_pairing_expires 
             ON pairing_requests(expires_at)",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS authorized_users (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                channel TEXT NOT NULL,
                user_id TEXT NOT NULL,
                username TEXT,
                approved_at INTEGER NOT NULL,
                UNIQUE(channel, user_id)
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_auth_channel_user 
             ON authorized_users(channel, user_id)",
            [],
        )?;

        Ok(())
    })
    .await
}

pub async fn insert_pairing_request(
    channel: &str,
    user_id: &str,
    username: Option<&str>,
    code: &str,
    created_at: i64,
    expires_at: i64,
) -> Result<()> {
    let channel = channel.to_string();
    let user_id = user_id.to_string();
    let username = username.map(|s| s.to_string());
    let code = code.to_string();
    run_db(move |conn| {
        conn.execute(
            "INSERT INTO pairing_requests (channel, user_id, username, code, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![channel, user_id, username, code, created_at, expires_at],
        )?;
        Ok(())
    })
    .await
}

pub async fn is_user_authorized(channel: &str, user_id: &str) -> Result<bool> {
    let channel = channel.to_string();
    let user_id = user_id.to_string();
    run_db(move |conn| {
        let mut stmt = conn
            .prepare("SELECT COUNT(*) FROM authorized_users WHERE channel = ?1 AND user_id = ?2")?;
        let count: i64 = stmt.query_row(params![channel, user_id], |row| row.get(0))?;
        Ok(count > 0)
    })
    .await
}

pub async fn get_user_pending_code(channel: &str, user_id: &str) -> Result<Option<String>> {
    let channel = channel.to_string();
    let user_id = user_id.to_string();
    run_db(move |conn| {
        let mut stmt =
            conn.prepare("SELECT code FROM pairing_requests WHERE channel = ?1 AND user_id = ?2")?;
        let result = stmt.query_row(params![channel, user_id], |row| row.get(0));

        match result {
            Ok(code) => Ok(Some(code)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    })
    .await
}

pub async fn get_pending_requests_for_channel(channel: &str) -> Result<Vec<PairingRequest>> {
    let channel = channel.to_string();
    run_db(move |conn| {
        let mut stmt = conn.prepare(
            "SELECT channel, user_id, username, code, created_at, expires_at 
             FROM pairing_requests WHERE channel = ?1 ORDER BY created_at DESC",
        )?;

        let rows = stmt.query_map(params![channel], |row| {
            Ok(PairingRequest {
                channel: row.get(0)?,
                user_id: row.get(1)?,
                username: row.get(2)?,
                code: row.get(3)?,
                created_at: row.get(4)?,
                expires_at: row.get(5)?,
            })
        })?;

        let mut requests = Vec::new();
        for row in rows {
            requests.push(row?);
        }

        Ok(requests)
    })
    .await
}

pub async fn get_all_pending_requests() -> Result<Vec<PairingRequest>> {
    run_db(move |conn| {
        let mut stmt = conn.prepare(
            "SELECT channel, user_id, username, code, created_at, expires_at 
             FROM pairing_requests ORDER BY channel, created_at DESC",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(PairingRequest {
                channel: row.get(0)?,
                user_id: row.get(1)?,
                username: row.get(2)?,
                code: row.get(3)?,
                created_at: row.get(4)?,
                expires_at: row.get(5)?,
            })
        })?;

        let mut requests = Vec::new();
        for row in rows {
            requests.push(row?);
        }

        Ok(requests)
    })
    .await
}

pub async fn get_request_by_code(channel: &str, code: &str) -> Result<Option<PairingRequest>> {
    let channel = channel.to_string();
    let code = code.to_string();
    run_db(move |conn| {
        let mut stmt = conn.prepare(
            "SELECT channel, user_id, username, code, created_at, expires_at 
             FROM pairing_requests WHERE channel = ?1 AND code = ?2",
        )?;

        let result = stmt.query_row(params![channel, code], |row| {
            Ok(PairingRequest {
                channel: row.get(0)?,
                user_id: row.get(1)?,
                username: row.get(2)?,
                code: row.get(3)?,
                created_at: row.get(4)?,
                expires_at: row.get(5)?,
            })
        });

        match result {
            Ok(req) => Ok(Some(req)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    })
    .await
}

pub async fn add_authorized_user(
    channel: &str,
    user_id: &str,
    username: Option<&str>,
    approved_at: i64,
) -> Result<()> {
    let channel = channel.to_string();
    let user_id = user_id.to_string();
    let username = username.map(|s| s.to_string());
    run_db(move |conn| {
        conn.execute(
            "INSERT OR REPLACE INTO authorized_users (channel, user_id, username, approved_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![channel, user_id, username, approved_at],
        )?;
        Ok(())
    })
    .await
}

pub async fn delete_pairing_request(channel: &str, code: &str) -> Result<usize> {
    let channel = channel.to_string();
    let code = code.to_string();
    run_db(move |conn| {
        let deleted = conn.execute(
            "DELETE FROM pairing_requests WHERE channel = ?1 AND code = ?2",
            params![channel, code],
        )?;
        Ok(deleted)
    })
    .await
}

pub async fn delete_expired_requests(now: i64) -> Result<usize> {
    run_db(move |conn| {
        let deleted = conn.execute(
            "DELETE FROM pairing_requests WHERE expires_at < ?1",
            params![now],
        )?;
        Ok(deleted)
    })
    .await
}
