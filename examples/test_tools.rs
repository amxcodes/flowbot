// Test the tools module

use flowbot_rs::tools::filesystem::*;
use flowbot_rs::tools::websearch::*;
use flowbot_rs::tools::commands::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("=== Testing File System Tools ===\n");
    
    // Test read_file
    println!("1. Reading test_file.txt...");
    let content = read_file(ReadFileArgs {
        path: "test_file.txt".to_string(),
    })
    .await?;
    println!("✓ Content: {}\n", content);
    
    // Test write_file
    println!("2. Writing new_file.txt...");
    let result = write_file(WriteFileArgs {
        path: "new_file.txt".to_string(),
        content: "Hello from Nanobot!".to_string(),
        overwrite: true,
    })
    .await?;
    println!("✓ {}\n", result);
    
    // Test list_directory
    println!("3. Listing current directory...");
    let files = list_directory(ListDirArgs {
        path: ".".to_string(),
        max_depth: Some(1),
    })
    .await?;
    println!("✓ Found {} items:", files.len());
    for file in files.iter().take(5) {
        println!("  - {} ({})", file.name, if file.is_dir { "dir" } else { "file" });
    }
    println!();
    
    // Test web_search
    println!("=== Testing Web Search ===\n");
    println!("4. Searching for 'Rust programming'...");
    match web_search(WebSearchArgs {
        query: "Rust programming".to_string(),
        max_results: 3,
    })
    .await
    {
        Ok(results) => {
            println!("✓ Found {} results:", results.len());
            for (i, result) in results.iter().enumerate() {
                println!("  {}. {}", i + 1, result.title);
                println!("     {}", result.url);
            }
        }
        Err(e) => println!("✗ Search failed: {}", e),
    }
    println!();
    
    // Test run_command
    println!("=== Testing Command Execution ===\n");
    println!("5. Running 'cargo --version'...");
    match run_command(RunCommandArgs {
        command: "cargo".to_string(),
        args: vec!["--version".to_string()],
        timeout_secs: 5,
    })
    .await
    {
        Ok(output) => {
            println!("✓ Command succeeded:");
            println!("  {}", output.stdout.trim());
        }
        Err(e) => println!("✗ Command failed: {}", e),
    }
    
    println!("\n=== All Tests Complete! ===");
    
    Ok(())
}
