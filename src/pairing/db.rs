use anyhow::Result;
use rusqlite::{Connection, params};
use std::sync::{Arc, Mutex};
use once_cell::sync::Lazy;

static DB_CONN: Lazy<Arc<Mutex<Connection>>> = Lazy::new(|| {
    let conn = Connection::open("flowbot.db").expect("Failed to open database");
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
    let conn = DB_CONN.lock().unwrap();
    
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
}

pub async fn insert_pairing_request(
    channel: &str,
    user_id: &str,
    username: Option<&str>,
    code: &str,
    created_at: i64,
    expires_at: i64,
) -> Result<()> {
    let conn = DB_CONN.lock().unwrap();
    conn.execute(
        "INSERT INTO pairing_requests (channel, user_id, username, code, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![channel, user_id, username, code, created_at, expires_at],
    )?;
    Ok(())
}

pub async fn is_user_authorized(channel: &str, user_id: &str) -> Result<bool> {
    let conn = DB_CONN.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT COUNT(*) FROM authorized_users WHERE channel = ?1 AND user_id = ?2"
    )?;
    
    let count: i64 = stmt.query_row(params![channel, user_id], |row| row.get(0))?;
    Ok(count > 0)
}

pub async fn get_user_pending_code(channel: &str, user_id: &str) -> Result<Option<String>> {
    let conn = DB_CONN.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT code FROM pairing_requests WHERE channel = ?1 AND user_id = ?2"
    )?;
    
    let result = stmt.query_row(params![channel, user_id], |row| row.get(0));
    
    match result {
        Ok(code) => Ok(Some(code)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub async fn get_pending_requests_for_channel(channel: &str) -> Result<Vec<PairingRequest>> {
    let conn = DB_CONN.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT channel, user_id, username, code, created_at, expires_at 
         FROM pairing_requests WHERE channel = ?1 ORDER BY created_at DESC"
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
}

pub async fn get_all_pending_requests() -> Result<Vec<PairingRequest>> {
    let conn = DB_CONN.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT channel, user_id, username, code, created_at, expires_at 
         FROM pairing_requests ORDER BY channel, created_at DESC"
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
}

pub async fn get_request_by_code(channel: &str, code: &str) -> Result<Option<PairingRequest>> {
    let conn = DB_CONN.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT channel, user_id, username, code, created_at, expires_at 
         FROM pairing_requests WHERE channel = ?1 AND code = ?2"
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
}

pub async fn add_authorized_user(
    channel: &str,
    user_id: &str,
    username: Option<&str>,
    approved_at: i64,
) -> Result<()> {
    let conn = DB_CONN.lock().unwrap();
    conn.execute(
        "INSERT OR REPLACE INTO authorized_users (channel, user_id, username, approved_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![channel, user_id, username, approved_at],
    )?;
    Ok(())
}

pub async fn delete_pairing_request(channel: &str, code: &str) -> Result<usize> {
    let conn = DB_CONN.lock().unwrap();
    let deleted = conn.execute(
        "DELETE FROM pairing_requests WHERE channel = ?1 AND code = ?2",
        params![channel, code],
    )?;
    Ok(deleted)
}

pub async fn delete_expired_requests(now: i64) -> Result<usize> {
    let conn = DB_CONN.lock().unwrap();
    let deleted = conn.execute(
        "DELETE FROM pairing_requests WHERE expires_at < ?1",
        params![now],
    )?;
    Ok(deleted)
}
