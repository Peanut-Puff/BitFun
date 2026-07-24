use bitfun_core::service::config::{
    get_global_config_service, subscribe_config_updates, ConfigUpdateEvent, GlobalConfig,
};
use bitfun_observability_otel::{
    EnvironmentTelemetrySecrets, TelemetryEntrypoint, TelemetryRuntime, TelemetryRuntimeMetadata,
};
use std::path::PathBuf;
use std::sync::Arc;

pub(crate) async fn initialize(state_directory: PathBuf) -> TelemetryRuntime {
    let runtime = TelemetryRuntime::new(
        TelemetryRuntimeMetadata::new(
            "bitfun-desktop",
            env!("CARGO_PKG_VERSION"),
            TelemetryEntrypoint::Desktop,
            state_directory,
        ),
        Arc::new(EnvironmentTelemetrySecrets),
    );
    apply_current_config(&runtime).await;
    runtime
}

pub(crate) fn spawn_config_listener(runtime: TelemetryRuntime) {
    tokio::spawn(async move {
        let Some(mut receiver) = subscribe_config_updates() else {
            log::warn!("Telemetry config subscription is unavailable");
            return;
        };

        loop {
            match receiver.recv().await {
                Ok(ConfigUpdateEvent::AppUpdated) | Ok(ConfigUpdateEvent::ConfigReloaded) => {
                    apply_current_config(&runtime).await;
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                    log::warn!("Telemetry config listener lagged by {count} events");
                    apply_current_config(&runtime).await;
                }
            }
        }
    });
}

async fn apply_current_config(runtime: &TelemetryRuntime) {
    let config_service = match get_global_config_service().await {
        Ok(service) => service,
        Err(error) => {
            log::warn!("Telemetry remains disabled because config is unavailable: {error}");
            return;
        }
    };
    let config = match config_service.get_config::<GlobalConfig>(None).await {
        Ok(config) => config.app.telemetry,
        Err(error) => {
            log::warn!("Telemetry remains disabled because config could not be read: {error}");
            return;
        }
    };

    if let Err(error) = runtime.apply_config(config) {
        log::warn!("Telemetry configuration was rejected and telemetry is disabled: {error}");
    }
}
