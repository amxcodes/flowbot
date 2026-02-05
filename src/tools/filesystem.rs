// File system operations tool

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::validate_path;

const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024; // 10MB limit

/// Arguments for reading a file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadFileArgs {
    pub path: String,
}

/// Read a file and return its contents
pub async fn read_file(args: ReadFileArgs) -> Result<String> {
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
pub struct WriteFileArgs {
    pub path: String,
    pub content: String,
    #[serde(default)]
    pub overwrite: bool,
}

/// Write content to a file
pub async fn write_file(args: WriteFileArgs) -> Result<String> {
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
    
    Ok(format!("Successfully wrote {} bytes to {}", args.content.len(), args.path))
}

/// Arguments for editing a file (find and replace)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditFileArgs {
    pub path: String,
    pub old_text: String,
    pub new_text: String,
    #[serde(default)]
    pub all_occurrences: bool,
}

/// Edit a file by finding and replacing text
pub async fn edit_file(args: EditFileArgs) -> Result<String> {
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
        return Err(anyhow::anyhow!("Text '{}' not found in file", args.old_text));
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
pub struct ListDirArgs {
    pub path: String,
    #[serde(default)]
    pub max_depth: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size: Option<u64>,
}

/// List files in a directory
pub async fn list_directory(args: ListDirArgs) -> Result<Vec<FileInfo>> {
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
