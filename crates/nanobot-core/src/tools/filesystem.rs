// File system operations tool

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::validate_path;

const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024; // 10MB limit

/// Arguments for reading a file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ReadFileArgs {
    pub path: String,
}

/// Read a file and return its contents
pub(super) async fn read_file(_token: &super::ExecutorToken, args: ReadFileArgs) -> Result<String> {
    let path = validate_path(&args.path)?;

    // Check file exists
    if !path.exists() {
        return Err(anyhow::anyhow!("File does not exist: {}", args.path));
    }

    if !path.is_file() {
        return Err(anyhow::anyhow!("Path is not a file: {}", args.path));
    }

    // Check file size
    let metadata = tokio::fs::metadata(&path).await?;
    if metadata.len() > MAX_FILE_SIZE {
        return Err(anyhow::anyhow!(
            "File too large: {} bytes (max: {} bytes)",
            metadata.len(),
            MAX_FILE_SIZE
        ));
    }

    // Read file
    let content = tokio::fs::read_to_string(&path).await?;

    Ok(content)
}

/// Arguments for writing a file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WriteFileArgs {
    pub path: String,
    pub content: String,
    #[serde(default)]
    pub overwrite: bool,
}

/// Write content to a file
pub(super) async fn write_file(_token: &super::ExecutorToken, args: WriteFileArgs) -> Result<String> {
    let path = validate_path(&args.path)?;

    // Check if file exists and overwrite flag
    if path.exists() && !args.overwrite {
        return Err(anyhow::anyhow!(
            "File already exists: {}. Set overwrite=true to replace.",
            args.path
        ));
    }

    // Create parent directories if needed
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    // Write file
    tokio::fs::write(&path, &args.content).await?;

    Ok(format!(
        "Successfully wrote {} bytes to {}",
        args.content.len(),
        args.path
    ))
}

/// Arguments for editing a file (find and replace)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct EditFileArgs {
    pub path: String,
    pub old_text: String,
    pub new_text: String,
    #[serde(default)]
    pub all_occurrences: bool,
}

/// Edit a file by finding and replacing text
pub(super) async fn edit_file(_token: &super::ExecutorToken, args: EditFileArgs) -> Result<String> {
    let path = validate_path(&args.path)?;

    // Read file
    let content = tokio::fs::read_to_string(&path).await?;

    // Perform replacement
    let new_content = if args.all_occurrences {
        content.replace(&args.old_text, &args.new_text)
    } else {
        content.replacen(&args.old_text, &args.new_text, 1)
    };

    // Check if anything was replaced
    if content == new_content {
        return Err(anyhow::anyhow!(
            "Text '{}' not found in file",
            args.old_text
        ));
    }

    let replacements = if args.all_occurrences {
        content.matches(&args.old_text).count()
    } else {
        1
    };

    // Write back
    tokio::fs::write(&path, &new_content).await?;

    Ok(format!(
        "Successfully replaced {} occurrence(s) in {}",
        replacements, args.path
    ))
}

/// Arguments for listing a directory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ListDirArgs {
    pub path: String,
    #[serde(default)]
    pub max_depth: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct FileInfo {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PatchOperation {
    Add {
        path: String,
        content: String,
        #[serde(default)]
        overwrite: bool,
    },
    Update {
        path: String,
        old_text: String,
        new_text: String,
        #[serde(default)]
        all_occurrences: bool,
        #[serde(default)]
        before_context: Option<String>,
        #[serde(default)]
        after_context: Option<String>,
    },
    Delete {
        path: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ApplyPatchArgs {
    #[serde(default)]
    pub patch: Option<String>,
    #[serde(default)]
    pub patch_text: Option<String>,
    #[serde(default)]
    pub operations: Vec<PatchOperation>,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub atomic: bool,
}

/// List files in a directory
pub(super) async fn list_directory(_token: &super::ExecutorToken, args: ListDirArgs) -> Result<Vec<FileInfo>> {
    let path = validate_path(&args.path)?;

    if !path.exists() {
        return Err(anyhow::anyhow!("Directory does not exist: {}", args.path));
    }

    if !path.is_dir() {
        return Err(anyhow::anyhow!("Path is not a directory: {}", args.path));
    }

    let max_depth = args.max_depth.unwrap_or(1);
    let mut files = Vec::new();

    fn walk_dir(
        path: &PathBuf,
        current_depth: usize,
        max_depth: usize,
        files: &mut Vec<FileInfo>,
    ) -> Result<()> {
        if current_depth > max_depth {
            return Ok(());
        }

        let entries = std::fs::read_dir(path)?;

        for entry in entries {
            let entry = entry?;
            let entry_path = entry.path();
            let metadata = entry.metadata()?;

            let file_info = FileInfo {
                path: entry_path.to_string_lossy().to_string(),
                name: entry.file_name().to_string_lossy().to_string(),
                is_dir: metadata.is_dir(),
                size: if metadata.is_file() {
                    Some(metadata.len())
                } else {
                    None
                },
            };

            files.push(file_info);

            // Recurse into subdirectories
            if metadata.is_dir() && current_depth < max_depth {
                walk_dir(&entry_path, current_depth + 1, max_depth, files)?;
            }
        }

        Ok(())
    }

    walk_dir(&path, 0, max_depth, &mut files)?;

    // Sort by name
    files.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(files)
}

/// Apply structured patch operations (add/update/delete)
pub(super) async fn apply_patch(_token: &super::ExecutorToken, args: ApplyPatchArgs) -> Result<String> {
    let unified_patch = args
        .patch
        .as_deref()
        .or(args.patch_text.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    if let Some(patch_text) = unified_patch {
        if args.dry_run {
            return Ok("dry-run unified diff parsing supported; execution skipped".to_string());
        }
        return apply_unified_diff_with_git(patch_text);
    }

    if args.operations.is_empty() {
        return Err(anyhow::anyhow!(
            "operations cannot be empty (or provide patch/patch_text)"
        ));
    }
    if args.operations.len() > 500 {
        return Err(anyhow::anyhow!("Too many operations (max 500)"));
    }

    let mut results = Vec::new();

    if args.dry_run {
        for (idx, op) in args.operations.iter().enumerate() {
            match op {
                PatchOperation::Add {
                    path, overwrite, ..
                } => {
                    let validated = validate_path(path)?;
                    if validated.exists() && !overwrite {
                        return Err(anyhow::anyhow!(
                            "dry-run op {} failed: add target exists and overwrite=false ({})",
                            idx,
                            path
                        ));
                    }
                    results.push(format!("dry-run add {}: ok", path));
                }
                PatchOperation::Update {
                    path,
                    old_text,
                    new_text,
                    all_occurrences,
                    before_context,
                    after_context,
                } => {
                    let validated = validate_path(path)?;
                    let content = tokio::fs::read_to_string(&validated).await.map_err(|e| {
                        anyhow::anyhow!("dry-run op {} failed to read {}: {}", idx, path, e)
                    })?;
                    let status = update_preview_status(
                        &content,
                        old_text,
                        new_text,
                        *all_occurrences,
                        before_context.as_deref(),
                        after_context.as_deref(),
                    )?;
                    results.push(format!("dry-run update {}: {}", path, status));
                }
                PatchOperation::Delete { path } => {
                    let validated = validate_path(path)?;
                    if validated.exists() {
                        if validated.is_file() {
                            results.push(format!("dry-run delete {}: remove file", path));
                        } else if validated.is_dir() {
                            results.push(format!("dry-run delete {}: remove directory", path));
                        } else {
                            results.push(format!("dry-run delete {}: unknown path type", path));
                        }
                    } else {
                        results.push(format!("dry-run delete {}: path not found (noop)", path));
                    }
                }
            }
        }
        return Ok(results.join("\n"));
    }

    if args.atomic {
        return apply_patch_atomic(args.operations).await;
    }

    for op in args.operations {
        match op {
            PatchOperation::Add {
                path,
                content,
                overwrite,
            } => {
                let validated = validate_path(&path)?;
                if validated.exists() && !overwrite {
                    return Err(anyhow::anyhow!(
                        "File already exists: {}. Set overwrite=true to replace.",
                        path
                    ));
                }
                if let Some(parent) = validated.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                tokio::fs::write(&validated, &content).await?;
                let msg = format!(
                    "Successfully wrote {} bytes to {}",
                    content.len(),
                    path
                );
                results.push(format!("add {}: {}", path, msg));
            }
            PatchOperation::Update {
                path,
                old_text,
                new_text,
                all_occurrences,
                before_context,
                after_context,
            } => {
                let validated = validate_path(&path)?;
                let content = tokio::fs::read_to_string(&validated).await?;
                let (new_content, status_msg) = apply_update_content(
                    &content,
                    &old_text,
                    &new_text,
                    all_occurrences,
                    before_context.as_deref(),
                    after_context.as_deref(),
                )?;
                if let Some(next) = new_content {
                    tokio::fs::write(&validated, next).await?;
                }
                results.push(format!("update {}: {}", path, status_msg));
            }
            PatchOperation::Delete { path } => {
                let validated = validate_path(&path)?;
                if validated.exists() {
                    if validated.is_file() {
                        tokio::fs::remove_file(&validated).await?;
                        results.push(format!("delete {}: removed file", path));
                    } else if validated.is_dir() {
                        tokio::fs::remove_dir_all(&validated).await?;
                        results.push(format!("delete {}: removed directory", path));
                    } else {
                        results.push(format!("delete {}: skipped unknown path type", path));
                    }
                } else {
                    results.push(format!("delete {}: path not found, skipped", path));
                }
            }
        }
    }

    Ok(results.join("\n"))
}

fn apply_unified_diff_with_git(patch_text: &str) -> Result<String> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    if Command::new("git").arg("--version").output().is_err() {
        return Err(anyhow::anyhow!(
            "Unified diff apply requires git executable in PATH"
        ));
    }

    let mut child = Command::new("git")
        .args(["apply", "--whitespace=nowarn", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("Failed to start git apply: {}", e))?;

    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(patch_text.as_bytes())
            .map_err(|e| anyhow::anyhow!("Failed to write patch stdin: {}", e))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|e| anyhow::anyhow!("Failed waiting for git apply: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("git apply failed: {}", stderr.trim()));
    }

    Ok("unified diff applied via git apply".to_string())
}

enum UndoAction {
    RemovePath(PathBuf),
    RestoreFile(PathBuf, Vec<u8>),
    RestoreDir { original: PathBuf, backup: PathBuf },
}

async fn apply_patch_atomic(operations: Vec<PatchOperation>) -> Result<String> {
    let mut results = Vec::new();
    let mut undo = Vec::<UndoAction>::new();

    for (idx, op) in operations.into_iter().enumerate() {
        let outcome: Result<String> = async {
            match op {
                PatchOperation::Add {
                    path,
                    content,
                    overwrite,
                } => {
                    let validated = validate_path(&path)?;
                    if validated.exists() {
                        if !validated.is_file() {
                            return Err(anyhow::anyhow!("add target is not a file: {}", path));
                        }
                        if !overwrite {
                            return Err(anyhow::anyhow!(
                                "add target exists and overwrite=false: {}",
                                path
                            ));
                        }
                        let original = tokio::fs::read(&validated).await?;
                        undo.push(UndoAction::RestoreFile(validated.clone(), original));
                    } else {
                        undo.push(UndoAction::RemovePath(validated.clone()));
                    }

                    if let Some(parent) = validated.parent() {
                        tokio::fs::create_dir_all(parent).await?;
                    }
                    tokio::fs::write(&validated, content).await?;
                    Ok(format!("add {}: ok", path))
                }
                PatchOperation::Update {
                    path,
                    old_text,
                    new_text,
                    all_occurrences,
                    before_context,
                    after_context,
                } => {
                    let validated = validate_path(&path)?;
                    let content = tokio::fs::read_to_string(&validated).await?;
                    let (new_content, status_msg) = apply_update_content(
                        &content,
                        &old_text,
                        &new_text,
                        all_occurrences,
                        before_context.as_deref(),
                        after_context.as_deref(),
                    )?;

                    if let Some(next) = new_content {
                        undo.push(UndoAction::RestoreFile(
                            validated.clone(),
                            content.into_bytes(),
                        ));
                        tokio::fs::write(&validated, next).await?;
                    }

                    Ok(format!("update {}: {}", path, status_msg))
                }
                PatchOperation::Delete { path } => {
                    let validated = validate_path(&path)?;
                    if !validated.exists() {
                        return Ok(format!("delete {}: path not found (noop)", path));
                    }

                    if validated.is_file() {
                        let original = tokio::fs::read(&validated).await?;
                        undo.push(UndoAction::RestoreFile(validated.clone(), original));
                        tokio::fs::remove_file(&validated).await?;
                        Ok(format!("delete {}: removed file", path))
                    } else if validated.is_dir() {
                        let backup = std::env::temp_dir()
                            .join(format!("nanobot_patch_backup_{}", uuid::Uuid::new_v4()));
                        tokio::fs::rename(&validated, &backup).await?;
                        undo.push(UndoAction::RestoreDir {
                            original: validated,
                            backup,
                        });
                        Ok(format!("delete {}: removed directory", path))
                    } else {
                        Ok(format!("delete {}: unknown path type (noop)", path))
                    }
                }
            }
        }
        .await;

        match outcome {
            Ok(msg) => results.push(msg),
            Err(e) => {
                rollback_undo_actions(&mut undo).await;
                return Err(anyhow::anyhow!(
                    "atomic apply failed at operation {}: {}. changes rolled back",
                    idx,
                    e
                ));
            }
        }
    }

    cleanup_undo_artifacts(&undo).await;
    Ok(results.join("\n"))
}

async fn rollback_undo_actions(undo: &mut Vec<UndoAction>) {
    while let Some(action) = undo.pop() {
        match action {
            UndoAction::RemovePath(path) => {
                let _ = if path.is_dir() {
                    tokio::fs::remove_dir_all(path).await
                } else {
                    tokio::fs::remove_file(path).await
                };
            }
            UndoAction::RestoreFile(path, bytes) => {
                if let Some(parent) = path.parent() {
                    let _ = tokio::fs::create_dir_all(parent).await;
                }
                let _ = tokio::fs::write(path, bytes).await;
            }
            UndoAction::RestoreDir { original, backup } => {
                let _ = tokio::fs::rename(backup, original).await;
            }
        }
    }
}

async fn cleanup_undo_artifacts(undo: &[UndoAction]) {
    for action in undo {
        if let UndoAction::RestoreDir { backup, .. } = action {
            let _ = tokio::fs::remove_dir_all(backup).await;
        }
    }
}

fn update_preview_status(
    content: &str,
    old_text: &str,
    new_text: &str,
    all_occurrences: bool,
    before_context: Option<&str>,
    after_context: Option<&str>,
) -> Result<String> {
    let (_, msg) = apply_update_content(
        content,
        old_text,
        new_text,
        all_occurrences,
        before_context,
        after_context,
    )?;
    Ok(msg)
}

fn apply_update_content(
    content: &str,
    old_text: &str,
    new_text: &str,
    all_occurrences: bool,
    before_context: Option<&str>,
    after_context: Option<&str>,
) -> Result<(Option<String>, String)> {
    if old_text.is_empty() {
        return Err(anyhow::anyhow!("old_text cannot be empty"));
    }

    let (search_start, search_end) =
        resolve_context_window(content, before_context, after_context)?;
    let segment = &content[search_start..search_end];

    if segment.contains(old_text) {
        let updated_segment = if all_occurrences {
            segment.replace(old_text, new_text)
        } else {
            segment.replacen(old_text, new_text, 1)
        };
        let mut updated =
            String::with_capacity(content.len() - segment.len() + updated_segment.len());
        updated.push_str(&content[..search_start]);
        updated.push_str(&updated_segment);
        updated.push_str(&content[search_end..]);

        let count = if all_occurrences {
            segment.matches(old_text).count()
        } else {
            1
        };

        if before_context.is_some() || after_context.is_some() {
            return Ok((
                Some(updated),
                format!("replaced {} occurrence(s) in constrained context", count),
            ));
        }

        let updated = if all_occurrences {
            content.replace(old_text, new_text)
        } else {
            content.replacen(old_text, new_text, 1)
        };
        let count = if all_occurrences {
            content.matches(old_text).count()
        } else {
            1
        };
        return Ok((Some(updated), format!("replaced {} occurrence(s)", count)));
    }

    if segment.contains(new_text) {
        return Ok((None, "noop (already applied)".to_string()));
    }

    if before_context.is_some() || after_context.is_some() {
        Err(anyhow::anyhow!(
            "Text '{}' not found in constrained context window",
            old_text
        ))
    } else {
        Err(anyhow::anyhow!("Text '{}' not found in file", old_text))
    }
}

fn resolve_context_window(
    content: &str,
    before_context: Option<&str>,
    after_context: Option<&str>,
) -> Result<(usize, usize)> {
    let mut start = 0usize;
    let mut end = content.len();

    if let Some(before) = before_context {
        if before.len() > 1000 {
            return Err(anyhow::anyhow!("before_context too large (max 1000 chars)"));
        }
        let pos = content
            .find(before)
            .ok_or_else(|| anyhow::anyhow!("before_context not found"))?;
        start = pos + before.len();
    }

    if let Some(after) = after_context {
        if after.len() > 1000 {
            return Err(anyhow::anyhow!("after_context too large (max 1000 chars)"));
        }
        let tail = &content[start..];
        let rel = tail
            .find(after)
            .ok_or_else(|| anyhow::anyhow!("after_context not found"))?;
        end = start + rel;
    }

    if start > end || end > content.len() {
        return Err(anyhow::anyhow!("invalid context window"));
    }

    Ok((start, end))
}
