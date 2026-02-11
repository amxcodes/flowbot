use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: String,
    pub content: String,
    pub status: String,
    pub priority: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoWriteArgs {
    pub todos: Vec<TodoItem>,
}

pub async fn todo_write(args: TodoWriteArgs, tenant_id: Option<&str>) -> Result<String> {
    if args.todos.is_empty() {
        return Err(anyhow::anyhow!("todos cannot be empty"));
    }

    let file_path = todo_file_path(tenant_id)?;
    if let Some(parent) = file_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let json = serde_json::to_string_pretty(&args.todos)?;
    tokio::fs::write(&file_path, json).await?;

    let pending = args
        .todos
        .iter()
        .filter(|t| t.status.eq_ignore_ascii_case("pending"))
        .count();
    let in_progress = args
        .todos
        .iter()
        .filter(|t| t.status.eq_ignore_ascii_case("in_progress"))
        .count();
    let completed = args
        .todos
        .iter()
        .filter(|t| t.status.eq_ignore_ascii_case("completed"))
        .count();
    let cancelled = args
        .todos
        .iter()
        .filter(|t| t.status.eq_ignore_ascii_case("cancelled"))
        .count();

    Ok(serde_json::json!({
        "status": "ok",
        "path": file_path.to_string_lossy(),
        "counts": {
            "total": args.todos.len(),
            "pending": pending,
            "in_progress": in_progress,
            "completed": completed,
            "cancelled": cancelled,
        }
    })
    .to_string())
}

fn todo_file_path(tenant_id: Option<&str>) -> Result<std::path::PathBuf> {
    let base = std::env::current_dir()?.join(".nanobot").join("todos");
    let name = sanitize_tenant(tenant_id.unwrap_or("default"));
    Ok(base.join(format!("{}.json", name)))
}

fn sanitize_tenant(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "default".to_string()
    } else {
        out
    }
}
