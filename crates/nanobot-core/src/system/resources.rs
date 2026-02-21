use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use sysinfo::System;
use tokio::time;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceUsage {
    pub cpu_usage_percent: f32,
    pub used_memory_mb: u64,
    pub total_memory_mb: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceLevel {
    High,   // Abundant resources
    Medium, // Moderate resources
    Low,    // Constrained resources
}

#[derive(Debug, Clone)]
pub struct AdaptiveConfig {
    pub max_tokens: u64, // Changed to u64 for LLM API compatibility
    pub rag_doc_count: usize,
    pub context_history_limit: usize,
}

pub struct ResourceMonitor {
    system: Arc<Mutex<System>>,
}

impl Default for ResourceMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl ResourceMonitor {
    pub fn new() -> Self {
        Self {
            system: Arc::new(Mutex::new(System::new_all())),
        }
    }

    pub async fn start_monitoring(&self) {
        let system = self.system.clone();
        tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_secs(5));
            loop {
                interval.tick().await;
                let mut sys = system.lock().unwrap();
                sys.refresh_cpu();
                sys.refresh_memory();
            }
        });
    }

    pub fn get_usage(&self) -> ResourceUsage {
        let sys = self.system.lock().unwrap();
        // Refresh called periodically, but we can also force refresh here if needed
        // sys.refresh_cpu(); // Blocking call

        let cpu_usage = sys.global_cpu_info().cpu_usage();
        let total_memory = sys.total_memory() / 1024 / 1024;
        let used_memory = sys.used_memory() / 1024 / 1024;

        ResourceUsage {
            cpu_usage_percent: cpu_usage,
            used_memory_mb: used_memory,
            total_memory_mb: total_memory,
        }
    }

    pub fn check_health(&self) -> bool {
        let usage = self.get_usage();
        // Simple heuristic: if free memory < 100MB, warn/fail
        let free_memory = usage.total_memory_mb - usage.used_memory_mb;
        if free_memory < 100 {
            tracing::warn!("⚠️ Low memory warning: {}MB free", free_memory);
            return false;
        }
        true
    }

    /// Determine current resource availability level
    pub fn get_resource_level(&self) -> ResourceLevel {
        let usage = self.get_usage();
        let free_memory = usage.total_memory_mb - usage.used_memory_mb;
        let memory_usage_percent =
            (usage.used_memory_mb as f32 / usage.total_memory_mb as f32) * 100.0;

        // Conservative thresholds for adaptive behavior
        if free_memory < 512 || memory_usage_percent > 85.0 || usage.cpu_usage_percent > 80.0 {
            ResourceLevel::Low
        } else if free_memory < 2048
            || memory_usage_percent > 70.0
            || usage.cpu_usage_percent > 60.0
        {
            ResourceLevel::Medium
        } else {
            ResourceLevel::High
        }
    }

    /// Get adaptive configuration based on current resources
    pub fn get_adaptive_config(&self) -> AdaptiveConfig {
        match self.get_resource_level() {
            ResourceLevel::High => AdaptiveConfig {
                max_tokens: 8192, // Parity with OpenClaw default
                rag_doc_count: 5,
                context_history_limit: 20,
            },
            ResourceLevel::Medium => AdaptiveConfig {
                max_tokens: 4096, // Fallback similar to Bedrock discovery
                rag_doc_count: 3,
                context_history_limit: 10,
            },
            ResourceLevel::Low => AdaptiveConfig {
                max_tokens: 2048, // Conservative for low resource
                rag_doc_count: 1,
                context_history_limit: 5,
            },
        }
    }
}
