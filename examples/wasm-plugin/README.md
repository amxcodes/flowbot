# Example WASM Plugin for Nanobot

This directory contains an example WASM plugin written in Rust that can be loaded by Nanobot.

## Building

1. **Install wasm32-wasi target**:
   ```bash
   rustup target add wasm32-wasi
   ```

2. **Build the plugin**:
   ```bash
   cargo build --target wasm32-wasi --release
   ```

3. **Output**:
   ```
   target/wasm32-wasi/release/example_wasm_tool.wasm
   ```

## Installation

1. **Create plugin directory**:
   ```bash
   mkdir -p ~/.config/nanobot/plugins/wasm-example
   ```

2. **Copy WASM file**:
   ```bash
   cp target/wasm32-wasi/release/example_wasm_tool.wasm \
      ~/.config/nanobot/plugins/wasm-example/tool.wasm
   ```

3. **Create manifest** (`~/.config/nanobot/plugins/wasm-example/nanobot.plugin.toml`):
   ```toml
   [plugin]
   id = "wasm-example"
   name = "WASM Example Tool"
   version = "1.0.0"
   description = "Example WASM-based tool"
   
   [[tools]]
   name = "wasm_example"
   enabled = true
   wasm_module = "tool.wasm"
   ```

4. **Reload plugins**:
   ```bash
   nanobot plugin reload
   ```

## Usage

```bash
nanobot chat
> Use the wasm_example tool to say hello
```

## Plugin Interface

WASM tools must export these functions:

### 1. `alloc(size: i32) -> i32`

Allocates memory of given size and returns pointer.

### 2. `execute(args_ptr: i32, args_len: i32) -> i32`

Executes the tool with JSON args and returns result pointer.

**Args**: JSON string with tool parameters  
**Returns**: Pointer to JSON result string (null-terminated)

## Example Implementation

See `src/lib.rs` for the full implementation.
