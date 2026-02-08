# Example: Web Research Pipeline
# Demonstrates chaining tools together

log("Starting web research pipeline...");

// 1. Search for information
let search_results = tool_search_web("Rust async programming best practices");
log("Search completed");

// 2. Read relevant file from workspace
let existing_notes = tool_read_file("notes/rust-async.md");

// 3. Combine and process
let combined = existing_notes + "\n\n## New Research\n" + search_results;

// 4. Write back to file
tool_write_file("notes/rust-async.md", combined);

log("Pipeline completed successfully!");
combined
