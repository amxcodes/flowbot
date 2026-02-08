use anyhow::Result;

pub mod admin;

pub use admin::start_admin_server;

pub async fn run_server(port: u16) -> Result<()> {
    println!("Server feature not yet implemented.");
    println!("Port: {}", port);
    Ok(())
}
