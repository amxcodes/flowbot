# Testing Nanobot Tools

## Quick Tool Tests

Run these commands to test the tools manually:

### 1. Test File Read Tool
```bash
.\target\release\nanobot-rs.exe agent -m "Read the demo.txt file" -p openrouter
```

### 2. Test List Directory
```bash
.\target\release\nanobot-rs.exe agent -m "List all files in the current directory" -p openrouter
```

### 3. Test Web Search
```bash
.\target\release\nanobot-rs.exe agent -m "Search for Rust programming tutorials" -p openrouter
```

### 4. Test Command Execution
```bash
.\target\release\nanobot-rs.exe agent -m "Run cargo --version" -p openrouter
```

### 5. Test File Write
```bash
.\target\release\nanobot-rs.exe agent -m "Create a new file called greeting.txt with the text 'Hello from Nanobot!'" -p openrouter
```

## TUI Testing

For interactive testing with tools:
```bash
.\target\release\nanobot-rs.exe chat -p openrouter
```

Then try asking:
- "What files are in this directory?"
- "Read the Cargo.toml file"
- "Search for information about Rig framework"
- "Run cargo --version for me"

## Expected Behavior

The agent should:
1. Recognize when it needs to use a tool
2. Output a JSON tool call (you'll see: 🔧 Using tool...)
3. Execute the tool (you'll see: ✓ Tool result...)
4. Continue the conversation with the result

## Notes

- Tools are available in both CLI mode (`agent`) and TUI mode (`chat`)
- The agent has a 5-iteration limit for tool calling
- Failed tool calls are shown and explained to the agent
- Path security prevents access to system directories
