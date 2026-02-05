use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Returns all available FlowBot tools (inspired by pi-coding-agent)
pub fn get_tool_declarations() -> Vec<ToolDefinition> {
    vec![
        // Tool 1: read_file - Read file contents
        ToolDefinition {
            name: "read_file".to_string(),
            description: "Read the contents of a file".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to read"
                    }
                },
                "required": ["path"]
            }),
        },
        // Tool 2: write_file - Create or overwrite a file
        ToolDefinition {
            name: "write_file".to_string(),
            description: "Create or overwrite a file with new content".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to write"
                    },
                    "content": {
                        "type": "string",
                        "description": "Content to write to the file"
                    },
                    "overwrite": {
                        "type": "boolean",
                        "description": "Overwrite if file exists (default: false)"
                    }
                },
                "required": ["path", "content"]
            }),
        },
        // Tool 3: edit_file - Make precise edits to a file
        ToolDefinition {
            name: "edit_file".to_string(),
            description: "Make precise edits to a file by replacing text".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to edit"
                    },
                    "old_text": {
                        "type": "string",
                        "description": "Exact text to find and replace"
                    },
                    "new_text": {
                        "type": "string",
                        "description": "New text to insert"
                    },
                    "all_occurrences": {
                        "type": "boolean",
                        "description": "Replace all occurrences (default: false)"
                    }
                },
                "required": ["path", "old_text", "new_text"]
            }),
        },
        // Tool 4: list_directory - List files in a directory
        ToolDefinition {
            name: "list_directory".to_string(),
            description: "List files in a directory".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory path to list"
                    },
                    "max_depth": {
                        "type": "integer",
                        "description": "Maximum recursion depth (default: 1)"
                    }
                },
                "required": ["path"]
            }),
        },
        // Tool 5: web_search - Search the web
        ToolDefinition {
            name: "web_search".to_string(),
            description: "Search the web".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query"
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum number of results (default: 5)"
                    }
                },
                "required": ["query"]
            }),
        },
        // Tool 6: run_command - Execute system commands
        ToolDefinition {
            name: "run_command".to_string(),
            description: "Execute a system command and return the output".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Command to execute"
                    },
                    "args": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Command arguments"
                    },
                    "use_docker": {
                        "type": "boolean",
                        "description": "Run in a container (default: false)"
                    }
                },
                "required": ["command"]
            }),
        },
        // Tool 7: spawn_process - Start a background process
        ToolDefinition {
            name: "spawn_process".to_string(),
            description: "Start a background process and return its PID".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Command to execute"
                    },
                    "args": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Command arguments"
                    }
                },
                "required": ["command"]
            }),
        },
        // Tool 8: read_process_output - Read output from a background process
        ToolDefinition {
            name: "read_process_output".to_string(),
            description: "Read and clear the output buffer of a background process".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pid": {
                        "type": "string",
                        "description": "Process ID (PID) returned by spawn_process"
                    }
                },
                "required": ["pid"]
            }),
        },
        // Tool 9: kill_process - Terminate a background process
        ToolDefinition {
            name: "kill_process".to_string(),
            description: "Terminate a background process".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pid": {
                        "type": "string",
                        "description": "Process ID (PID) returned by spawn_process"
                    }
                },
                "required": ["pid"]
            }),
        },
        // Tool 10: list_processes - List all background processes
        ToolDefinition {
            name: "list_processes".to_string(),
            description: "List all running background processes".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        // Tool 11: web_fetch - Download and extract content from a URL
        ToolDefinition {
            name: "web_fetch".to_string(),
            description: "Download content from a URL and extract text (strips HTML)".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "URL to fetch"
                    }
                },
                "required": ["url"]
            }),
        },
        // Tool 12: write_process_input - Send input to stdin
        ToolDefinition {
            name: "write_process_input".to_string(),
            description: "Send text input to a running process (useful for interactive prompts)".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pid": {
                        "type": "string",
                        "description": "Process ID"
                    },
                    "input": {
                        "type": "string",
                        "description": "Input text to send (include newline \\n if needed)"
                    }
                },
                "required": ["pid", "input"]
            }),
        },
        // Tool 13: cron - Manage scheduled tasks
        ToolDefinition {
            name: "cron".to_string(),
            description: "Manage cron jobs for time-based task automation (status/list/add/update/remove/run/runs/wake)".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "description": "Action to perform",
                        "enum": ["status", "list", "add", "update", "remove", "run", "runs", "wake"]
                    },
                    "includeDisabled": {
                        "type": "boolean",
                        "description": "Include disabled jobs in list (default: false)"
                    },
                    "job": {
                        "type": "object",
                        "description": "Job definition for 'add' action"
                    },
                    "jobId": {
                        "type": "string",
                        "description": "Job ID for update/remove/run/runs actions"
                    },
                    "patch": {
                        "type": "object",
                        "description": "Job updates for 'update' action"
                    },
                    "text": {
                        "type": "string",
                        "description": "Wake event text for 'wake' action"
                    },
                    "mode": {
                        "type": "string",
                        "description": "Wake mode: 'now' or 'next-heartbeat'",
                        "enum": ["now", "next-heartbeat"]
                    }
                },
                "required": ["action"]
            }),
        },
        // Tool 14: sessions_spawn - Create isolated subagent
        ToolDefinition {
            name: "sessions_spawn".to_string(),
            description: "Spawn a background sub-agent in an isolated session to handle a specific task".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "task": {
                        "type": "string",
                        "description": "Task description for the subagent"
                    },
                    "label": {
                        "type": "string",
                        "description": "Human-readable label for the subagent (optional)"
                    },
                    "model": {
                        "type": "string",
                        "description": "Model override for subagent (optional)"
                    },
                    "cleanup": {
                        "type": "string",
                        "description": "Cleanup policy: 'keep' or 'delete' (default: keep)",
                        "enum": ["keep", "delete"]
                    }
                },
                "required": ["task"]
            }),
        },
    ]





}

/// Convert tool definitions to Gemini API function_declarations format
pub fn to_gemini_tools(tools: Vec<ToolDefinition>) -> serde_json::Value {
    json!([{
        "function_declarations": tools.iter().map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters
            })
        }).collect::<Vec<_>>()
    }])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_declarations() {
        let tools = get_tool_declarations();
        assert_eq!(tools.len(), 14);
        assert_eq!(tools[0].name, "read_file");
        // ...
        assert_eq!(tools[11].name, "write_process_input");
        assert_eq!(tools[12].name, "cron");
        assert_eq!(tools[13].name, "sessions_spawn");
    }

    #[test]
    fn test_gemini_format() {
        let tools = get_tool_declarations();
        let gemini_tools = to_gemini_tools(tools);

        // Verify structure
        assert!(gemini_tools.is_array());
        assert!(gemini_tools[0]["function_declarations"].is_array());
        assert_eq!(
            gemini_tools[0]["function_declarations"]
                .as_array()
                .unwrap()
                .len(),
            14
        );
    }
}
