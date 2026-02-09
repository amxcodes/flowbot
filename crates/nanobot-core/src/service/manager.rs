use anyhow::Result;

use super::types::ServiceRuntime;

/// Platform-agnostic service manager
pub struct ServiceManager;

impl ServiceManager {
    pub fn new() -> Self {
        ServiceManager
    }
    
    /// Install the service
    pub fn install(&self) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            super::systemd::install()
        }
        
        #[cfg(target_os = "windows")]
        {
            super::windows::install()
        }
        
        #[cfg(target_os = "macos")]
        {
            anyhow::bail!("macOS launchd support not yet implemented. Use 'nanobot gateway' manually for now.")
        }
        
        #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
        {
            anyhow::bail!("Service installation not supported on this platform")
        }
    }
    
    /// Uninstall the service
    pub fn uninstall(&self) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            super::systemd::uninstall()
        }
        
        #[cfg(target_os = "windows")]
        {
            super::windows::uninstall()
        }
        
        #[cfg(target_os = "macos")]
        {
            anyhow::bail!("macOS launchd support not yet implemented")
        }
        
        #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
        {
            anyhow::bail!("Service uninstallation not supported on this platform")
        }
    }
    
    /// Start the service
    pub fn start(&self) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            super::systemd::start()
        }
        
        #[cfg(target_os = "windows")]
        {
            super::windows::start()
        }
        
        #[cfg(target_os = "macos")]
        {
            anyhow::bail!("macOS launchd support not yet implemented")
        }
        
        #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
        {
            anyhow::bail!("Service start not supported on this platform")
        }
    }
    
    /// Stop the service
    pub fn stop(&self) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            super::systemd::stop()
        }
        
        #[cfg(target_os = "windows")]
        {
            super::windows::stop()
        }
        
        #[cfg(target_os = "macos")]
        {
            anyhow::bail!("macOS launchd support not yet implemented")
        }
        
        #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
        {
            anyhow::bail!("Service stop not supported on this platform")
        }
    }
    
    /// Restart the service
    pub fn restart(&self) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            super::systemd::restart()
        }
        
        #[cfg(target_os = "windows")]
        {
            super::windows::restart()
        }
        
        #[cfg(target_os = "macos")]
        {
            anyhow::bail!("macOS launchd support not yet implemented")
        }
        
        #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
        {
            anyhow::bail!("Service restart not supported on this platform")
        }
    }
    
    /// Get service status
    pub fn status(&self) -> Result<ServiceRuntime> {
        #[cfg(target_os = "linux")]
        {
            super::systemd::status()
        }
        
        #[cfg(target_os = "windows")]
        {
            super::windows::status()
        }
        
        #[cfg(target_os = "macos")]
        {
            anyhow::bail!("macOS launchd support not yet implemented")
        }
        
        #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
        {
            anyhow::bail!("Service status not supported on this platform")
        }
    }
    
    /// Check if service is installed
    pub fn is_installed(&self) -> bool {
        #[cfg(target_os = "linux")]
        {
            super::systemd::is_installed()
        }
        
        #[cfg(target_os = "windows")]
        {
            super::windows::is_installed()
        }
        
        #[cfg(target_os = "macos")]
        {
            false
        }
        
        #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
        {
            false
        }
    }
}

impl Default for ServiceManager {
    fn default() -> Self {
        Self::new()
    }
}
