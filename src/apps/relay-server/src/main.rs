//! BitFun Relay Server
//!
//! Standalone binary that runs the relay as a network service.
//! Uses `DiskAssetStore` for filesystem-backed mobile-web file storage.

use anyhow::Context;
use bitfun_observability_otel::{
    deployment_config_from_env, EnvironmentTelemetrySecrets, TelemetryEntrypoint, TelemetryRuntime,
    TelemetryRuntimeMetadata,
};
use std::sync::Arc;
use tracing::info;

mod config;

use bitfun_relay_service::{DiskAssetStore, RoomManager, WebAssetStore};
use config::RelayConfig;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cfg = RelayConfig::from_env();
    info!("BitFun Relay Server v{}", env!("CARGO_PKG_VERSION"));
    let telemetry_runtime = initialize_telemetry(&cfg);
    let startup_observation = bitfun_observability::domains::start_startup(
        &telemetry_runtime.telemetry(),
        bitfun_observability::domains::StartupStartFacts {
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            platform: bitfun_observability::domains::current_platform_class(),
            entrypoint: bitfun_observability::domains::Entrypoint::Relay,
            phase: bitfun_observability::domains::StartupPhase::Runtime,
            state: bitfun_observability::domains::RuntimeState::Started,
        },
        None,
    );

    let room_manager = RoomManager::new();
    let asset_store = Arc::new(DiskAssetStore::new_with_max_bytes(
        &cfg.room_web_dir,
        cfg.asset_store_max_bytes,
    ));

    let cleanup_rm = room_manager.clone();
    let cleanup_ttl = cfg.room_ttl_secs;
    let cleanup_store = asset_store.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            let stale_ids = cleanup_rm.cleanup_stale_rooms(cleanup_ttl);
            for room_id in &stale_ids {
                cleanup_store.cleanup_room(room_id);
            }
        }
    });

    let start_time = std::time::Instant::now();

    let db = if let Some(path) = &cfg.db_path {
        let pool = bitfun_relay_service::db::connect(path)
            .await
            .with_context(|| {
                format!("failed to initialize configured account database at {path}")
            })?;
        Some(Arc::new(pool))
    } else {
        info!("RELAY_DB_PATH not set — account features disabled (pure relay mode)");
        None
    };
    if db.is_some() && cfg.cors_allow_origins.iter().any(|origin| origin == "*") {
        anyhow::bail!(
            "RELAY_CORS_ALLOW_ORIGINS=* is not allowed when RELAY_DB_PATH enables account APIs"
        );
    }
    let page_browser_auth = match (
        cfg.page_public_base_url.as_deref(),
        cfg.page_auth_base_url.as_deref(),
    ) {
        (Some(public_base_url), Some(auth_base_url)) => Some(
            bitfun_relay_service::PageBrowserAuthConfig::new(public_base_url, auth_base_url)
                .map_err(anyhow::Error::msg)?,
        ),
        (None, None) => {
            if db.is_some() {
                tracing::warn!(
                    "RELAY_PAGE_PUBLIC_BASE_URL and RELAY_PAGE_AUTH_BASE_URL are not set; \
                     protected Page login uses same-origin compatibility mode"
                );
            }
            None
        }
        _ => anyhow::bail!(
            "RELAY_PAGE_PUBLIC_BASE_URL and RELAY_PAGE_AUTH_BASE_URL must be configured together"
        ),
    };

    let page_data_dir = std::path::PathBuf::from(&cfg.room_web_dir).join("page-data");
    let mut app = bitfun_relay_service::build_relay_router_with_page_data_origins_and_page_auth(
        room_manager,
        asset_store,
        start_time,
        db,
        env!("CARGO_PKG_VERSION"),
        Some(page_data_dir),
        cfg.cors_allow_origins.clone(),
        page_browser_auth,
    );

    if let Some(static_dir) = &cfg.static_dir {
        info!("Serving static files from: {static_dir}");
        app = app.fallback_service(
            tower_http::services::ServeDir::new(static_dir).append_index_html_on_directories(true),
        );
    }
    // Re-apply after installing the optional fallback so static files receive
    // the same browser hardening as relay API responses.
    app = app.layer(axum::middleware::from_fn(
        bitfun_relay_service::relay_security_headers,
    ));

    info!("Room web upload dir: {}", cfg.room_web_dir);
    info!("Asset store capacity: {} bytes", cfg.asset_store_max_bytes);

    let listener = tokio::net::TcpListener::bind(cfg.listen_addr).await?;
    startup_observation.finish(bitfun_observability::domains::StartupFinishFacts {
        completion: bitfun_observability::domains::CompletionFacts::completed(),
    });
    info!("Relay server listening on {}", cfg.listen_addr);
    info!("WebSocket endpoint: ws://{}/ws", cfg.listen_addr);

    let serve_result = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await;
    if let Err(error) = telemetry_runtime.shutdown() {
        tracing::warn!("Telemetry shutdown did not complete: {error}");
    }
    serve_result?;
    Ok(())
}

fn initialize_telemetry(config: &RelayConfig) -> TelemetryRuntime {
    let runtime = TelemetryRuntime::new(
        TelemetryRuntimeMetadata::new(
            "bitfun-relay-server",
            env!("CARGO_PKG_VERSION"),
            TelemetryEntrypoint::Relay,
            &config.telemetry_state_dir,
        ),
        Arc::new(EnvironmentTelemetrySecrets),
    );
    match deployment_config_from_env("BITFUN_TELEMETRY") {
        Ok(config) => {
            if let Err(error) = runtime.apply_config(config) {
                tracing::warn!(
                    "Telemetry deployment configuration was rejected and telemetry is disabled: {error}"
                );
            }
        }
        Err(error) => tracing::warn!(
            "Telemetry deployment configuration is invalid and telemetry is disabled: {error}"
        ),
    }
    runtime
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::warn!("Could not install the Ctrl-C handler: {error}");
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::warn!("Could not install the termination signal handler: {error}");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(unix)]
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    #[cfg(not(unix))]
    ctrl_c.await;
}
