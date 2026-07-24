use bitfun_core::service::config::{get_global_config_service, GlobalConfig};
use bitfun_observability_otel::{
    EnvironmentTelemetrySecrets, TelemetryEntrypoint, TelemetryRuntime, TelemetryRuntimeMetadata,
};
use std::sync::Arc;

pub(crate) struct ShutdownGuard(TelemetryRuntime);

impl ShutdownGuard {
    pub(crate) fn new(runtime: TelemetryRuntime) -> Self {
        Self(runtime)
    }
}

impl Drop for ShutdownGuard {
    fn drop(&mut self) {
        if let Err(error) = self.0.shutdown() {
            tracing::warn!("Telemetry shutdown did not complete: {error}");
        }
    }
}

pub(crate) async fn initialize() -> TelemetryRuntime {
    let state_directory = bitfun_core::infrastructure::get_path_manager_arc().user_data_dir();
    let runtime = TelemetryRuntime::new(
        TelemetryRuntimeMetadata::new(
            "bitfun-cli",
            env!("CARGO_PKG_VERSION"),
            TelemetryEntrypoint::Cli,
            state_directory,
        ),
        Arc::new(EnvironmentTelemetrySecrets),
    );

    if let Err(error) = bitfun_core::service::config::initialize_global_config().await {
        tracing::warn!("Telemetry remains disabled because config initialization failed: {error}");
        return runtime;
    }
    let config_service = match get_global_config_service().await {
        Ok(service) => service,
        Err(error) => {
            tracing::warn!("Telemetry remains disabled because config is unavailable: {error}");
            return runtime;
        }
    };
    let config = match config_service.get_config::<GlobalConfig>(None).await {
        Ok(config) => config.app.telemetry,
        Err(error) => {
            tracing::warn!("Telemetry remains disabled because config could not be read: {error}");
            return runtime;
        }
    };
    if let Err(error) = runtime.apply_config(config) {
        tracing::warn!("Telemetry configuration was rejected and telemetry is disabled: {error}");
    }

    runtime
}
