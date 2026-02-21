use anyhow::Result;
use opentelemetry::KeyValue;
use opentelemetry::global;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::Tracer;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

/// Initialize tracing with OpenTelemetry support
pub fn init_tracing(service_name: &str, otlp_endpoint: Option<String>) -> Result<()> {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer());

    // Optionally add OpenTelemetry layer if OTLP endpoint is provided
    if let Some(endpoint) = otlp_endpoint {
        let tracer = init_otlp_tracer(service_name, &endpoint)?;
        let telemetry_layer = tracing_opentelemetry::layer().with_tracer(tracer);

        registry.with(telemetry_layer).init();

        tracing::info!(
            "OpenTelemetry tracing initialized with endpoint: {}",
            endpoint
        );
    } else {
        registry.init();
        tracing::info!("Basic tracing initialized (no OTLP export)");
    }

    Ok(())
}

/// Initialize OpenTelemetry OTLP exporter
fn init_otlp_tracer(service_name: &str, endpoint: &str) -> Result<Tracer> {
    use opentelemetry_otlp::WithExportConfig;

    let exporter = opentelemetry_otlp::new_exporter()
        .tonic()
        .with_endpoint(endpoint);

    let tracer = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(exporter)
        .with_trace_config(opentelemetry_sdk::trace::Config::default().with_resource(
            Resource::new(vec![KeyValue::new(
                "service.name",
                service_name.to_string(),
            )]),
        ))
        .install_batch(opentelemetry_sdk::runtime::Tokio)?;

    Ok(tracer)
}

/// Shutdown tracing (flush pending spans)
pub fn shutdown_tracing() {
    global::shutdown_tracer_provider();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_basic_tracing() {
        // Should not panic
        let _ = init_tracing("nanobot-test", None);
    }
}
