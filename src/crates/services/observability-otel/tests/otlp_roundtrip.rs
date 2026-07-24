use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::routing::post;
use axum::Router;
use bitfun_observability::config::{
    OtlpCompression, TelemetryBatchConfig, TelemetryConfig, TelemetryExporterConfig,
    TelemetrySamplingConfig,
};
use bitfun_observability::domains::{
    start_startup, CompletionFacts, Entrypoint, PlatformClass, RuntimeState, StartupFinishFacts,
    StartupPhase, StartupStartFacts,
};
use bitfun_observability::TelemetryLevel;
use bitfun_observability_otel::{
    DeploymentEnvironment, TelemetryEntrypoint, TelemetryRuntime, TelemetryRuntimeMetadata,
};
use opentelemetry_proto::tonic::collector::logs::v1::{
    ExportLogsServiceRequest, ExportLogsServiceResponse,
};
use opentelemetry_proto::tonic::collector::metrics::v1::{
    ExportMetricsServiceRequest, ExportMetricsServiceResponse,
};
use opentelemetry_proto::tonic::collector::trace::v1::{
    ExportTraceServiceRequest, ExportTraceServiceResponse,
};
use prost::Message;
use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::oneshot;

#[derive(Clone, Default)]
struct CollectorState {
    traces: Arc<AtomicUsize>,
    metrics: Arc<AtomicUsize>,
    logs: Arc<AtomicUsize>,
    invalid_content_type: Arc<AtomicUsize>,
}

impl CollectorState {
    fn assert_received_all_signals(&self) {
        assert!(self.traces.load(Ordering::Acquire) > 0, "no spans received");
        assert!(
            self.metrics.load(Ordering::Acquire) >= 2,
            "expected counter and histogram metrics"
        );
        assert!(self.logs.load(Ordering::Acquire) > 0, "no logs received");
        assert_eq!(self.invalid_content_type.load(Ordering::Acquire), 0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn otlp_http_protobuf_round_trips_all_three_signals() {
    let (state, endpoint, stop_tx, server) = start_http_collector().await;

    emit_one_operation(endpoint);
    state.assert_received_all_signals();

    let _ = stop_tx.send(());
    server.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn endpoint_reconfiguration_replaces_the_export_generation() {
    let (first_state, first_endpoint, first_stop, first_server) = start_http_collector().await;
    let (second_state, second_endpoint, second_stop, second_server) = start_http_collector().await;
    let temporary = tempfile::tempdir().unwrap();
    let runtime = test_runtime(temporary.path());

    runtime.apply_config(test_config(first_endpoint)).unwrap();
    emit_startup_fact(&runtime);
    runtime.force_flush().unwrap();
    first_state.assert_received_all_signals();

    runtime.apply_config(test_config(second_endpoint)).unwrap();
    emit_startup_fact(&runtime);
    runtime.shutdown().unwrap();
    second_state.assert_received_all_signals();
    assert_eq!(runtime.health().generation, 2);
    assert_eq!(runtime.health().reconfigurations, 2);

    let _ = first_stop.send(());
    let _ = second_stop.send(());
    first_server.await.unwrap();
    second_server.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn signal_switches_disable_only_the_selected_pipeline() {
    let (state, endpoint, stop_tx, server) = start_http_collector().await;
    let temporary = tempfile::tempdir().unwrap();
    let runtime = test_runtime(temporary.path());
    let mut config = test_config(endpoint);
    config.signals.traces = false;
    runtime.apply_config(config).unwrap();

    emit_startup_fact(&runtime);
    let before_shutdown = runtime.health();
    assert_eq!(before_shutdown.facade.accepted(), 3);
    assert_eq!(before_shutdown.pipeline.submitted, 3);
    runtime.shutdown().unwrap();

    assert_eq!(state.traces.load(Ordering::Acquire), 0);
    let metrics = state.metrics.load(Ordering::Acquire);
    let logs = state.logs.load(Ordering::Acquire);
    assert!(metrics >= 2, "expected two metrics, received {metrics}");
    assert!(logs > 0, "expected a log record, received {logs}");
    let _ = stop_tx.send(());
    server.await.unwrap();
}

#[test]
fn unreachable_exporter_is_contained_and_reported() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let temporary = tempfile::tempdir().unwrap();
    let runtime = test_runtime(temporary.path());
    runtime.apply_config(test_config(endpoint)).unwrap();

    emit_startup_fact(&runtime);

    assert!(runtime.shutdown().is_err());
    assert!(runtime.health().pipeline.export_failures > 0);
}

async fn start_http_collector() -> (
    CollectorState,
    String,
    oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let state = CollectorState::default();
    let app = Router::new()
        .route("/v1/traces", post(http_traces))
        .route("/v1/metrics", post(http_metrics))
        .route("/v1/logs", post(http_logs))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (stop_tx, stop_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stop_rx.await;
            })
            .await
            .unwrap();
    });
    (state, endpoint, stop_tx, server)
}

fn emit_one_operation(endpoint: String) {
    let temporary = tempfile::tempdir().unwrap();
    let runtime = test_runtime(temporary.path());
    runtime.apply_config(test_config(endpoint)).unwrap();
    emit_startup_fact(&runtime);
    runtime.shutdown().unwrap();
}

fn test_runtime(state_directory: &std::path::Path) -> TelemetryRuntime {
    TelemetryRuntime::without_secrets(
        TelemetryRuntimeMetadata::new(
            "bitfun-otlp-test",
            "0.0.0-test",
            TelemetryEntrypoint::Cli,
            state_directory,
        )
        .with_environment(DeploymentEnvironment::Test),
    )
}

fn test_config(endpoint: String) -> TelemetryConfig {
    TelemetryConfig {
        level: TelemetryLevel::Diagnostic,
        exporter: TelemetryExporterConfig {
            endpoint: Some(endpoint),
            compression: OtlpCompression::Gzip,
            allow_insecure_loopback: true,
            ..Default::default()
        },
        batch: TelemetryBatchConfig {
            max_queue_size: 32,
            max_export_batch_size: 8,
            scheduled_delay_ms: 10,
            metrics_export_interval_ms: 10,
            export_timeout_ms: 2_000,
            shutdown_timeout_ms: 2_000,
        },
        sampling: TelemetrySamplingConfig {
            diagnostic_trace_ratio: 1.0,
            diagnostic_success_log_ratio: 1.0,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn emit_startup_fact(runtime: &TelemetryRuntime) {
    let observation = start_startup(
        &runtime.telemetry(),
        StartupStartFacts {
            app_version: "0.0.0-test".to_string(),
            platform: PlatformClass::Other,
            entrypoint: Entrypoint::Cli,
            phase: StartupPhase::Bootstrap,
            state: RuntimeState::Started,
        },
        None,
    );
    observation.finish(StartupFinishFacts {
        completion: CompletionFacts::completed(),
    });
}

async fn http_traces(
    State(state): State<CollectorState>,
    headers: HeaderMap,
    body: Bytes,
) -> (StatusCode, [(header::HeaderName, &'static str); 1], Vec<u8>) {
    validate_content_type(&state, &headers);
    let request = ExportTraceServiceRequest::decode(decode_http_body(&headers, body)).unwrap();
    let count = request
        .resource_spans
        .iter()
        .flat_map(|resource| &resource.scope_spans)
        .map(|scope| scope.spans.len())
        .sum::<usize>();
    state.traces.fetch_add(count, Ordering::Release);
    protobuf_response(ExportTraceServiceResponse::default())
}

async fn http_metrics(
    State(state): State<CollectorState>,
    headers: HeaderMap,
    body: Bytes,
) -> (StatusCode, [(header::HeaderName, &'static str); 1], Vec<u8>) {
    validate_content_type(&state, &headers);
    let request = ExportMetricsServiceRequest::decode(decode_http_body(&headers, body)).unwrap();
    let count = request
        .resource_metrics
        .iter()
        .flat_map(|resource| &resource.scope_metrics)
        .map(|scope| scope.metrics.len())
        .sum::<usize>();
    state.metrics.fetch_add(count, Ordering::Release);
    protobuf_response(ExportMetricsServiceResponse::default())
}

async fn http_logs(
    State(state): State<CollectorState>,
    headers: HeaderMap,
    body: Bytes,
) -> (StatusCode, [(header::HeaderName, &'static str); 1], Vec<u8>) {
    validate_content_type(&state, &headers);
    let request = ExportLogsServiceRequest::decode(decode_http_body(&headers, body)).unwrap();
    let count = request
        .resource_logs
        .iter()
        .flat_map(|resource| &resource.scope_logs)
        .map(|scope| scope.log_records.len())
        .sum::<usize>();
    state.logs.fetch_add(count, Ordering::Release);
    protobuf_response(ExportLogsServiceResponse::default())
}

fn validate_content_type(state: &CollectorState, headers: &HeaderMap) {
    if headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        != Some("application/x-protobuf")
    {
        state.invalid_content_type.fetch_add(1, Ordering::Release);
    }
}

fn decode_http_body(headers: &HeaderMap, body: Bytes) -> Bytes {
    if headers
        .get(header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        == Some("gzip")
    {
        let mut decoded = Vec::new();
        flate2::read::GzDecoder::new(body.as_ref())
            .read_to_end(&mut decoded)
            .unwrap();
        Bytes::from(decoded)
    } else {
        body
    }
}

fn protobuf_response<T: Message>(
    response: T,
) -> (StatusCode, [(header::HeaderName, &'static str); 1], Vec<u8>) {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/x-protobuf")],
        response.encode_to_vec(),
    )
}
