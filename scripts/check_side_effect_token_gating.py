#!/usr/bin/env python3
from __future__ import annotations

import re
import sys
from pathlib import Path


SIDE_EFFECT_FILES = [
    Path("crates/nanobot-core/src/tools/filesystem.rs"),
    Path("crates/nanobot-core/src/tools/fetch.rs"),
    Path("crates/nanobot-core/src/tools/process.rs"),
    Path("crates/nanobot-core/src/tools/search.rs"),
    Path("crates/nanobot-core/src/tools/todos.rs"),
    Path("crates/nanobot-core/src/tools/commands.rs"),
    Path("crates/nanobot-core/src/tools/cli_wrapper.rs"),
]

DEFINITIONS_FILE = Path("crates/nanobot-core/src/tools/definitions.rs")

FORBIDDEN_REGISTRY_WRAPPERS = [
    "read_file_tool::ReadFileTool",
    "write_file_tool::WriteFileTool",
    "list_directory_tool::ListDirectoryTool",
    "edit_file_tool::EditFileTool",
    "spawn_process_tool::SpawnProcessTool",
    "read_process_output_tool::ReadProcessOutputTool",
    "kill_process_tool::KillProcessTool",
    "web_fetch_tool::WebFetchTool",
    "write_process_input_tool::WriteProcessInputTool",
]


FN_START_RE = re.compile(r"^pub(?:\([^)]*\))?\s+(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)")
TOKEN_RE = re.compile(r"&\s*(?:super::)?ExecutorToken\b")

ALLOWLIST_NO_TOKEN_FUNCTIONS = {
    "dangerous_command_detected",
    "command_allowed",
    "resolve_command",
    "format_output",
    "format_stream",
}


def collect_signature(lines: list[str], start_idx: int) -> tuple[str, int]:
    sig_lines = [lines[start_idx].rstrip("\n")]
    depth = lines[start_idx].count("(") - lines[start_idx].count(")")
    idx = start_idx
    while (depth > 0 or ("{" not in sig_lines[-1] and ";" not in sig_lines[-1])) and idx + 1 < len(lines):
        idx += 1
        line = lines[idx].rstrip("\n")
        sig_lines.append(line)
        depth += line.count("(") - line.count(")")
    return "\n".join(sig_lines), idx


def check_side_effect_signatures(repo_root: Path) -> list[str]:
    errors: list[str] = []

    for rel in SIDE_EFFECT_FILES:
        path = repo_root / rel
        if not path.exists():
            errors.append(f"Missing side-effect file: {rel}")
            continue

        lines = path.read_text(encoding="utf-8").splitlines(keepends=True)
        i = 0
        while i < len(lines):
            match = FN_START_RE.match(lines[i])
            if not match:
                i += 1
                continue

            fn_name = match.group(1)
            signature, end_idx = collect_signature(lines, i)
            if fn_name in ALLOWLIST_NO_TOKEN_FUNCTIONS:
                i = end_idx + 1
                continue
            if not TOKEN_RE.search(signature):
                line_no = i + 1
                errors.append(
                    f"{rel}:{line_no} public function '{fn_name}' missing '&ExecutorToken' gating"
                )
            i = end_idx + 1

    return errors


def check_registry_wrappers(repo_root: Path) -> list[str]:
    errors: list[str] = []
    path = repo_root / DEFINITIONS_FILE
    if not path.exists():
        return [f"Missing file: {DEFINITIONS_FILE}"]

    text = path.read_text(encoding="utf-8")
    for marker in FORBIDDEN_REGISTRY_WRAPPERS:
        if marker in text:
            errors.append(
                f"Forbidden side-effect wrapper registration detected in {DEFINITIONS_FILE}: {marker}"
            )
    return errors


def main() -> int:
    repo_root = Path(__file__).resolve().parents[1]
    errors = []
    errors.extend(check_side_effect_signatures(repo_root))
    errors.extend(check_registry_wrappers(repo_root))

    if errors:
        print("Side-effect token gating check FAILED:")
        for err in errors:
            print(f"- {err}")
        return 1

    print("Side-effect token gating check passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
