#[derive(Debug, thiserror::Error)]
pub enum TelemetryRuntimeError {
    #[error("telemetry configuration is invalid: {0}")]
    InvalidConfig(&'static str),
    #[error("telemetry secret resolution failed: {0}")]
    Secret(&'static str),
    #[error("telemetry identity storage failed")]
    Identity(#[source] std::io::Error),
    #[error("telemetry exporter setup failed for {signal}")]
    Exporter {
        signal: &'static str,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("telemetry runtime is already shut down")]
    AlreadyShutdown,
    #[error("telemetry flush or shutdown did not complete: {0}")]
    Lifecycle(&'static str),
}

impl TelemetryRuntimeError {
    pub(crate) fn exporter(
        signal: &'static str,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::Exporter {
            signal,
            source: Box::new(source),
        }
    }
}
