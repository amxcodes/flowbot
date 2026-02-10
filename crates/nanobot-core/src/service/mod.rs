// Service module for daemon/background operation
pub mod macos;
pub mod manager;
pub mod systemd;
pub mod types;
pub mod windows;

pub use manager::ServiceManager;
pub use types::{ServiceInfo, ServiceResponse, ServiceStatus};
