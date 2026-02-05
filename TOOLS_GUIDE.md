# Nanobot-rs Tools Guide

Nanobot allows the AI agent to interact with your system through a set of secure, predefined tools.

## Available Tools

### 1. File Operations

#### `read_file`
Reads the contents of a file.
- **Arguments**: `path` (string)
- **Example Agent Request**:
  ```json
  {"tool": "read_file", "path": "README.md"}
  ```

#### `write_file`
Writes content to a file. Creates parent directories if they don't exist.
- **Arguments**: 
  - `path` (string)
  - `content` (string)
  - `overwrite` (boolean, default: false)
- **Example Agent Request**:
  ```json
  {"tool": "write_file", "path": "notes.txt", "content": "Meeting notes...", "overwrite": true}
  ```

#### `edit_file`
Finds and replaces text in a file. Useful for code edits.
- **Arguments**:
  - `path` (string)
  - `old_text` (string)
  - `new_text` (string)
  - `all_occurrences` (boolean, default: false)
- **Example Agent Request**:
  ```json
  {"tool": "edit_file", "path": "config.toml", "old_text": "debug = false", "new_text": "debug = true"}
  ```

#### `list_directory`
Lists files and subdirectories in a folder.
- **Arguments**:
  - `path` (string)
  - `max_depth` (integer, default: 1)
- **Example Agent Request**:
  ```json
  {"tool": "list_directory", "path": "src", "max_depth": 2}
  ```

### 2. System Operations

#### `run_command`
Executes a system shell command. **Enhanced for VPS Administration**.
- **Allowed Commands**: 
  - **Core**: `cargo`, `git`, `npm`, `node`, `python`, `pip`, `ls`, `cd`, `pwd`, `echo`, `cat`, `grep`, `find`, `mkdir`, `cp`, `mv`, `rm`
  - **VPS Admin**: `systemctl`, `journalctl`, `service`, `uptime`, `df`, `free`, `ps`, `top`
  - **Network**: `curl`, `wget`, `ping`, `netstat`
  - **Containers**: `docker`, `docker-compose`
- **Arguments**:
  - `command` (string)
  - `args` (array of strings)
  - `timeout_secs` (integer, default: 30) - **NEW**: Set longer timeouts for builds/installs.
  - `background` (boolean, default: false) - **NEW**: Run without waiting (coming soon).
- **Example Agent Request**:
  ```json
  {"tool": "run_command", "command": "systemctl", "args": ["restart", "nginx"], "timeout_secs": 60}
  ```

### 3. Web

#### `web_search`
Performs a web search using DuckDuckGo (HTML scraping, no API key required).
- **Arguments**:
  - `query` (string)
  - `max_results` (integer, default: 5)
- **Example Agent Request**:
  ```json
  {"tool": "web_search", "query": "latest rust features", "max_results": 3}
  ```

## Security

- **Path Validation**: All file paths are validated to prevent directory traversal (`../`) and access to system directories (`/etc`, `C:\Windows`).
- **Command Whitelist**: Only specific, safe commands are allowed.
- **Timeouts**: Long-running commands are terminated after a timeout.

## Usage

The agent automatically decides when to use these tools based on your request.
- "Read the README file" -> `read_file`
- "Search for the latest news" -> `web_search`
- "List files in the current folder" -> `list_directory`
