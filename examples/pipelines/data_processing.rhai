# Example: Data Processing Pipeline
# Shows conditional logic and error handling

log("Starting data processing...");

// 1. Execute command to get data
let raw_data = tool_exec("cat data.json");

// 2. Check if data exists
if raw_data.contains("error") {
    log("ERROR: Failed to read data");
    return "Pipeline failed";
}

// 3. Process data (simulate parsing)
let processed = "Processed: " + raw_data;

// 4. Write to multiple outputs
tool_write_file("output/processed.txt", processed);
tool_write_file("output/backup.txt", raw_data);

log("Data processing complete");
"Success"
