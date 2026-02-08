---
name: github
description: "GitHub operations using gh CLI - manage issues, PRs, repos, and more"
category: automation
status: active
---

# GitHub Skill

Provides comprehensive GitHub operations using the official `gh` CLI tool.

## Prerequisites

Install the GitHub CLI:
```bash
# Windows (winget)
winget install --id GitHub.cli

# macOS
brew install gh

# Linux
sudo apt install gh  # or equivalent
```

Authenticate:
```bash
gh auth login
```

## Tools Provided

### `gh_issue_create`
Create a new issue in a repository.
- **Args**: `repo` (owner/name), `title`, `body`
- **Example**: `{"tool": "gh_issue_create", "repo": "me/myrepo", "title": "Bug fix", "body": "Description"}`

### `gh_issue_list`
List issues in a repository.
- **Args**: `repo` (owner/name), `state` (open/closed/all), `limit` (default: 10)
- **Example**: `{"tool": "gh_issue_list", "repo": "rust-lang/rust", "state": "open", "limit": 5}`

### `gh_pr_create`
Create a pull request.
- **Args**: `repo`, `title`, `body`, `base` (default: main), `head` (current branch)
- **Example**: `{"tool": "gh_pr_create", "repo": "me/myrepo", "title": "Add feature", "body": "Details"}`

### `gh_pr_list`
List pull requests.
- **Args**: `repo`, `state` (open/closed/merged/all), `limit`
- **Example**: `{"tool": "gh_pr_list", "repo": "torvalds/linux", "state": "open", "limit": 5}`

### `gh_repo_view`
View repository information.
- **Args**: `repo` (owner/name)
- **Example**: `{"tool": "gh_repo_view", "repo": "microsoft/vscode"}`

### `gh_repo_clone`
Clone a repository.
- **Args**: `repo`, `directory` (optional)
- **Example**: `{"tool": "gh_repo_clone", "repo": "facebook/react", "directory": "./react"}`

## Configuration

```toml
[skills.github]
enabled = true
gh_path = "gh"  # Path to gh CLI (auto-detected if in PATH)
```

## Usage Examples

**List open issues in a Rust project:**
```
> List the latest 5 open issues in rust-lang/rust
✓ Fetched 5 issues from rust-lang/rust
```

**Create an issue:**
```
> Create an issue in my repo myuser/myproject titled "Add dark mode" with body "Users requested dark mode support"
✓ Created issue #42 in myuser/myproject
```

**View a repository:**
```
> Show me info about the facebook/react repository
✓ Repository: facebook/react
  Description: The library for web and native user interfaces
  Stars: 220k | Forks: 45k
  Language: JavaScript
```

## Implementation Notes

- Uses `run_command` tool to execute `gh` CLI
- All operations require `gh auth login` to be run first
- Rate limits follow GitHub API limits (5000 req/hour for authenticated users)
- Supports both public and private repositories (based on auth)
