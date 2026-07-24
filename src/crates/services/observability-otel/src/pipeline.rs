use crate::settings::{TelemetryRuntimeMetadata, ValidatedTelemetrySettings};
use crate::TelemetryRuntimeError;
use bitfun_observability::config::{OtlpCompression, OtlpTransport};
use bitfun_observability::{
    descriptor_registry, Attribute, AttributeValue, LogRecord as BitFunLogRecord, MetricRecord,
    MetricUnit, MetricValue, Severity as BitFunSeverity, SpanContext as BitFunSpanContext,
    SpanRecord, SpanStatus as BitFunSpanStatus, ValidatedRecord,
};
use opentelemetry::logs::{AnyValue, LogRecord as _, Logger as _, LoggerProvider as _, Severity};
use opentelemetry::metrics::{Counter, Histogram, Meter, MeterProvider as _, UpDownCounter};
use opentelemetry::trace::{
    Link, SpanContext, SpanId, SpanKind, Status, TraceFlags, TraceId, TraceState,
};
use opentelemetry::{InstrumentationScope, KeyValue, Value};
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_otlp::{
    Compression, Protocol, WithExportConfig, WithHttpConfig, WithTonicConfig,
};
use opentelemetry_sdk::logs::{LogExporter, SdkLogger, SdkLoggerProvider};
use opentelemetry_sdk::metrics::{
    data::ResourceMetrics, exporter::PushMetricExporter, PeriodicReader, SdkMeterProvider,
    Temporality,
};
use opentelemetry_sdk::trace::{
    BatchConfigBuilder as TraceBatchConfigBuilder, BatchSpanProcessor, SpanData, SpanEvents,
    SpanExporter, SpanLinks, SpanProcessor,
};
use opentelemetry_sdk::Resource;
use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug)]
struct RestrictedHttpClient {
    inner: reqwest::blocking::Client,
    configured_headers: Vec<(reqwest::header::HeaderName, reqwest::header::HeaderValue)>,
}

#[async_trait::async_trait]
impl HttpClient for RestrictedHttpClient {
    async fn send_bytes(&self, mut request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        restrict_http_headers(&mut request, &self.configured_headers);
        self.inner.send_bytes(request).await
    }
}

fn restrict_http_headers(
    request: &mut Request<Bytes>,
    configured_headers: &[(reqwest::header::HeaderName, reqwest::header::HeaderValue)],
) {
    let generated_headers = ["content-type", "content-encoding", "user-agent"]
        .into_iter()
        .filter_map(|name| {
            request
                .headers()
                .get(name)
                .cloned()
                .map(|value| (reqwest::header::HeaderName::from_static(name), value))
        })
        .collect::<Vec<_>>();
    request.headers_mut().clear();
    for (name, value) in generated_headers {
        request.headers_mut().insert(name, value);
    }
    for (name, value) in configured_headers {
        request.headers_mut().insert(name.clone(), value.clone());
    }
}

#[derive(Debug, Default)]
pub(crate) struct PipelineCounters {
    submitted: AtomicU64,
    dropped_inactive: AtomicU64,
    dropped_signal_disabled: AtomicU64,
    exported_batches: AtomicU64,
    export_failures: AtomicU64,
    suppressed_batches: AtomicU64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PipelineDiagnostics {
    pub submitted: u64,
    pub dropped_inactive: u64,
    pub dropped_signal_disabled: u64,
    pub exported_batches: u64,
    pub export_failures: u64,
    pub suppressed_batches: u64,
}

impl PipelineCounters {
    fn snapshot(&self) -> PipelineDiagnostics {
        PipelineDiagnostics {
            submitted: self.submitted.load(Ordering::Relaxed),
            dropped_inactive: self.dropped_inactive.load(Ordering::Relaxed),
            dropped_signal_disabled: self.dropped_signal_disabled.load(Ordering::Relaxed),
            exported_batches: self.exported_batches.load(Ordering::Relaxed),
            export_failures: self.export_failures.load(Ordering::Relaxed),
            suppressed_batches: self.suppressed_batches.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug)]
struct ExportGate {
    active: AtomicBool,
    counters: Arc<PipelineCounters>,
}

impl ExportGate {
    fn new(counters: Arc<PipelineCounters>) -> Self {
        Self {
            active: AtomicBool::new(true),
            counters,
        }
    }

    fn deactivate(&self) {
        self.active.store(false, Ordering::Release);
    }

    fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    fn record_result<T>(&self, result: &Result<T, opentelemetry_sdk::error::OTelSdkError>) {
        if result.is_ok() {
            self.counters
                .exported_batches
                .fetch_add(1, Ordering::Relaxed);
        } else {
            self.counters
                .export_failures
                .fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[derive(Debug)]
struct GuardedSpanExporter<E> {
    inner: E,
    gate: Arc<ExportGate>,
}

impl<E> SpanExporter for GuardedSpanExporter<E>
where
    E: SpanExporter + Debug,
{
    async fn export(&self, batch: Vec<SpanData>) -> opentelemetry_sdk::error::OTelSdkResult {
        if !self.gate.is_active() {
            self.gate
                .counters
                .suppressed_batches
                .fetch_add(1, Ordering::Relaxed);
            return Ok(());
        }
        let result = self.inner.export(batch).await;
        self.gate.record_result(&result);
        result
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> opentelemetry_sdk::error::OTelSdkResult {
        self.inner.shutdown_with_timeout(timeout)
    }

    fn force_flush(&self) -> opentelemetry_sdk::error::OTelSdkResult {
        self.inner.force_flush()
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.inner.set_resource(resource);
    }
}

#[derive(Debug)]
struct GuardedLogExporter<E> {
    inner: E,
    gate: Arc<ExportGate>,
}

impl<E> LogExporter for GuardedLogExporter<E>
where
    E: LogExporter + Debug,
{
    async fn export(
        &self,
        batch: opentelemetry_sdk::logs::LogBatch<'_>,
    ) -> opentelemetry_sdk::error::OTelSdkResult {
        if !self.gate.is_active() {
            self.gate
                .counters
                .suppressed_batches
                .fetch_add(1, Ordering::Relaxed);
            return Ok(());
        }
        let result = self.inner.export(batch).await;
        self.gate.record_result(&result);
        result
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> opentelemetry_sdk::error::OTelSdkResult {
        self.inner.shutdown_with_timeout(timeout)
    }

    fn event_enabled(&self, level: Severity, target: &str, name: Option<&str>) -> bool {
        self.gate.is_active() && self.inner.event_enabled(level, target, name)
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.inner.set_resource(resource);
    }
}

struct GuardedMetricExporter<E> {
    inner: E,
    gate: Arc<ExportGate>,
}

impl<E> PushMetricExporter for GuardedMetricExporter<E>
where
    E: PushMetricExporter,
{
    async fn export(&self, metrics: &ResourceMetrics) -> opentelemetry_sdk::error::OTelSdkResult {
        if !self.gate.is_active() {
            self.gate
                .counters
                .suppressed_batches
                .fetch_add(1, Ordering::Relaxed);
            return Ok(());
        }
        let result = self.inner.export(metrics).await;
        self.gate.record_result(&result);
        result
    }

    fn force_flush(&self) -> opentelemetry_sdk::error::OTelSdkResult {
        self.inner.force_flush()
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> opentelemetry_sdk::error::OTelSdkResult {
        self.inner.shutdown_with_timeout(timeout)
    }

    fn temporality(&self) -> Temporality {
        self.inner.temporality()
    }
}

enum MetricInstrument {
    Counter(Counter<u64>),
    Histogram(Histogram<f64>),
    UpDownCounter(UpDownCounter<i64>),
}

struct MetricPipeline {
    provider: SdkMeterProvider,
    meter: Meter,
    instruments: Mutex<HashMap<&'static str, MetricInstrument>>,
}

impl MetricPipeline {
    fn record(&self, record: MetricRecord) {
        let attributes = otel_attributes(record.attributes(), None);
        let mut instruments = self
            .instruments
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let instrument = instruments.entry(record.name()).or_insert_with(|| {
            let unit = metric_unit(record.name());
            match record.value() {
                MetricValue::Counter(_) => MetricInstrument::Counter(
                    self.meter
                        .u64_counter(record.name())
                        .with_description("BitFun typed telemetry counter")
                        .with_unit(unit)
                        .build(),
                ),
                MetricValue::Histogram(_) => MetricInstrument::Histogram(
                    self.meter
                        .f64_histogram(record.name())
                        .with_description("BitFun typed telemetry histogram")
                        .with_unit(unit)
                        .build(),
                ),
                MetricValue::UpDownCounter(_) => MetricInstrument::UpDownCounter(
                    self.meter
                        .i64_up_down_counter(record.name())
                        .with_description("BitFun typed telemetry up-down counter")
                        .with_unit(unit)
                        .build(),
                ),
            }
        });

        match (instrument, record.value()) {
            (MetricInstrument::Counter(instrument), MetricValue::Counter(value)) => {
                instrument.add(*value, &attributes);
            }
            (MetricInstrument::Histogram(instrument), MetricValue::Histogram(value)) => {
                instrument.record(*value, &attributes);
            }
            (MetricInstrument::UpDownCounter(instrument), MetricValue::UpDownCounter(value)) => {
                instrument.add(*value, &attributes);
            }
            _ => {}
        }
    }
}

pub(crate) struct OtelGeneration {
    gate: Arc<ExportGate>,
    counters: Arc<PipelineCounters>,
    trace: Option<BatchSpanProcessor>,
    metrics: Option<MetricPipeline>,
    logs: Option<(SdkLoggerProvider, SdkLogger)>,
    scope: InstrumentationScope,
    shutdown_timeout: Duration,
    shutdown: AtomicBool,
}

impl Debug for OtelGeneration {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OtelGeneration")
            .field("active", &self.gate.is_active())
            .field("traces", &self.trace.is_some())
            .field("metrics", &self.metrics.is_some())
            .field("logs", &self.logs.is_some())
            .field("diagnostics", &self.diagnostics())
            .finish_non_exhaustive()
    }
}

impl OtelGeneration {
    pub(crate) fn build(
        settings: &ValidatedTelemetrySettings,
        metadata: &TelemetryRuntimeMetadata,
        scoped_installation_id: String,
        instance_id: &str,
        signals: bitfun_observability::config::TelemetrySignalConfig,
    ) -> Result<Self, TelemetryRuntimeError> {
        install_rustls_provider();
        let counters = Arc::new(PipelineCounters::default());
        let gate = Arc::new(ExportGate::new(counters.clone()));
        let resource = Resource::builder()
            .with_service_name(metadata.service_name)
            .with_attributes([
                KeyValue::new("service.version", metadata.service_version),
                KeyValue::new("service.instance.id", instance_id.to_string()),
                KeyValue::new("deployment.environment.name", metadata.environment.as_str()),
                KeyValue::new("bitfun.entrypoint", metadata.entrypoint.as_str()),
                KeyValue::new("bitfun.installation.id", scoped_installation_id),
                KeyValue::new("os.type", std::env::consts::OS),
                KeyValue::new("host.arch", std::env::consts::ARCH),
            ])
            .build();
        let scope = InstrumentationScope::builder("bitfun-observability")
            .with_version(env!("CARGO_PKG_VERSION"))
            .build();

        let trace = if signals.traces {
            let exporter = GuardedSpanExporter {
                inner: build_span_exporter(settings)?,
                gate: gate.clone(),
            };
            let config = TraceBatchConfigBuilder::default()
                .with_max_queue_size(settings.max_queue_size)
                .with_max_export_batch_size(settings.max_export_batch_size)
                .with_scheduled_delay(settings.scheduled_delay)
                .build();
            let mut processor = BatchSpanProcessor::builder(exporter)
                .with_batch_config(config)
                .build();
            processor.set_resource(&resource);
            Some(processor)
        } else {
            None
        };

        let metrics = if signals.metrics {
            let exporter = GuardedMetricExporter {
                inner: build_metric_exporter(settings)?,
                gate: gate.clone(),
            };
            let reader = PeriodicReader::builder(exporter)
                .with_interval(settings.metrics_export_interval)
                .build();
            let provider = SdkMeterProvider::builder()
                .with_reader(reader)
                .with_resource(resource.clone())
                .build();
            let meter = provider.meter_with_scope(scope.clone());
            Some(MetricPipeline {
                provider,
                meter,
                instruments: Mutex::new(HashMap::new()),
            })
        } else {
            None
        };

        let logs = if signals.logs {
            let exporter = GuardedLogExporter {
                inner: build_log_exporter(settings)?,
                gate: gate.clone(),
            };
            let batch = opentelemetry_sdk::logs::BatchConfigBuilder::default()
                .with_max_queue_size(settings.max_queue_size)
                .with_max_export_batch_size(settings.max_export_batch_size)
                .with_scheduled_delay(settings.scheduled_delay)
                .build();
            let processor = opentelemetry_sdk::logs::BatchLogProcessor::builder(exporter)
                .with_batch_config(batch)
                .build();
            let provider = SdkLoggerProvider::builder()
                .with_log_processor(processor)
                .with_resource(resource)
                .build();
            let logger = provider.logger_with_scope(scope.clone());
            Some((provider, logger))
        } else {
            None
        };

        Ok(Self {
            gate,
            counters,
            trace,
            metrics,
            logs,
            scope,
            shutdown_timeout: settings.shutdown_timeout,
            shutdown: AtomicBool::new(false),
        })
    }

    pub(crate) fn emit(&self, record: ValidatedRecord) {
        if !self.gate.is_active() || self.shutdown.load(Ordering::Acquire) {
            self.counters
                .dropped_inactive
                .fetch_add(1, Ordering::Relaxed);
            return;
        }
        self.counters.submitted.fetch_add(1, Ordering::Relaxed);
        match record {
            ValidatedRecord::Span(record) => match self.trace.as_ref() {
                Some(processor) => processor.on_end(span_data(record, self.scope.clone())),
                None => self.record_signal_disabled(),
            },
            ValidatedRecord::Metric(record) => match self.metrics.as_ref() {
                Some(pipeline) => pipeline.record(record),
                None => self.record_signal_disabled(),
            },
            ValidatedRecord::Log(record) => match self.logs.as_ref() {
                Some((_, logger)) => emit_log(logger, record),
                None => self.record_signal_disabled(),
            },
        }
    }

    pub(crate) fn deactivate(&self) {
        self.gate.deactivate();
    }

    pub(crate) fn diagnostics(&self) -> PipelineDiagnostics {
        self.counters.snapshot()
    }

    pub(crate) fn force_flush(&self) -> Result<(), TelemetryRuntimeError> {
        let mut failure = None;
        if let Some(processor) = &self.trace {
            if processor.force_flush().is_err() {
                failure.get_or_insert("trace force flush failed");
            }
        }
        if let Some(pipeline) = &self.metrics {
            if pipeline.provider.force_flush().is_err() {
                failure.get_or_insert("metric force flush failed");
            }
        }
        if let Some((provider, _)) = &self.logs {
            if provider.force_flush().is_err() {
                failure.get_or_insert("log force flush failed");
            }
        }
        match failure {
            Some(failure) => Err(TelemetryRuntimeError::Lifecycle(failure)),
            None => Ok(()),
        }
    }

    pub(crate) fn shutdown(&self, graceful: bool) -> Result<(), TelemetryRuntimeError> {
        if self.shutdown.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let mut failure = None;
        if !graceful {
            self.deactivate();
        }
        let deadline = Instant::now() + self.shutdown_timeout;
        if let Some(processor) = &self.trace {
            if processor
                .shutdown_with_timeout(deadline.saturating_duration_since(Instant::now()))
                .is_err()
            {
                failure.get_or_insert("trace shutdown failed");
            }
        }
        if let Some(pipeline) = &self.metrics {
            if pipeline
                .provider
                .shutdown_with_timeout(deadline.saturating_duration_since(Instant::now()))
                .is_err()
            {
                failure.get_or_insert("metric shutdown failed");
            }
        }
        if let Some((provider, _)) = &self.logs {
            if provider
                .shutdown_with_timeout(deadline.saturating_duration_since(Instant::now()))
                .is_err()
            {
                failure.get_or_insert("log shutdown failed");
            }
        }
        self.deactivate();
        match failure {
            Some(failure) => Err(TelemetryRuntimeError::Lifecycle(failure)),
            None => Ok(()),
        }
    }

    fn record_signal_disabled(&self) {
        self.counters
            .dropped_signal_disabled
            .fetch_add(1, Ordering::Relaxed);
    }
}

impl Drop for OtelGeneration {
    fn drop(&mut self) {
        self.gate.deactivate();
    }
}

fn build_span_exporter(
    settings: &ValidatedTelemetrySettings,
) -> Result<opentelemetry_otlp::SpanExporter, TelemetryRuntimeError> {
    match settings.transport {
        OtlpTransport::HttpProtobuf => {
            let builder = opentelemetry_otlp::SpanExporter::builder()
                .with_http()
                .with_endpoint(http_signal_endpoint(settings, "/v1/traces"))
                .with_protocol(Protocol::HttpBinary)
                .with_timeout(settings.export_timeout)
                .with_http_client(blocking_http_client(settings)?);
            let result = match settings.compression {
                OtlpCompression::None => builder.build(),
                OtlpCompression::Gzip => builder.with_compression(Compression::Gzip).build(),
            };
            result.map_err(|error| TelemetryRuntimeError::exporter("trace", error))
        }
        OtlpTransport::Grpc => {
            let builder = opentelemetry_otlp::SpanExporter::builder()
                .with_tonic()
                .with_endpoint(settings.endpoint.clone())
                .with_protocol(Protocol::Grpc)
                .with_timeout(settings.export_timeout)
                .with_interceptor(grpc_metadata_interceptor(&settings.headers)?);
            let result = match settings.compression {
                OtlpCompression::None => builder.build(),
                OtlpCompression::Gzip => builder.with_compression(Compression::Gzip).build(),
            };
            result.map_err(|error| TelemetryRuntimeError::exporter("trace", error))
        }
    }
}

fn build_metric_exporter(
    settings: &ValidatedTelemetrySettings,
) -> Result<opentelemetry_otlp::MetricExporter, TelemetryRuntimeError> {
    match settings.transport {
        OtlpTransport::HttpProtobuf => {
            let builder = opentelemetry_otlp::MetricExporter::builder()
                .with_http()
                .with_endpoint(http_signal_endpoint(settings, "/v1/metrics"))
                .with_protocol(Protocol::HttpBinary)
                .with_timeout(settings.export_timeout)
                .with_http_client(blocking_http_client(settings)?);
            let result = match settings.compression {
                OtlpCompression::None => builder.build(),
                OtlpCompression::Gzip => builder.with_compression(Compression::Gzip).build(),
            };
            result.map_err(|error| TelemetryRuntimeError::exporter("metric", error))
        }
        OtlpTransport::Grpc => {
            let builder = opentelemetry_otlp::MetricExporter::builder()
                .with_tonic()
                .with_endpoint(settings.endpoint.clone())
                .with_protocol(Protocol::Grpc)
                .with_timeout(settings.export_timeout)
                .with_interceptor(grpc_metadata_interceptor(&settings.headers)?);
            let result = match settings.compression {
                OtlpCompression::None => builder.build(),
                OtlpCompression::Gzip => builder.with_compression(Compression::Gzip).build(),
            };
            result.map_err(|error| TelemetryRuntimeError::exporter("metric", error))
        }
    }
}

fn build_log_exporter(
    settings: &ValidatedTelemetrySettings,
) -> Result<opentelemetry_otlp::LogExporter, TelemetryRuntimeError> {
    match settings.transport {
        OtlpTransport::HttpProtobuf => {
            let builder = opentelemetry_otlp::LogExporter::builder()
                .with_http()
                .with_endpoint(http_signal_endpoint(settings, "/v1/logs"))
                .with_protocol(Protocol::HttpBinary)
                .with_timeout(settings.export_timeout)
                .with_http_client(blocking_http_client(settings)?);
            let result = match settings.compression {
                OtlpCompression::None => builder.build(),
                OtlpCompression::Gzip => builder.with_compression(Compression::Gzip).build(),
            };
            result.map_err(|error| TelemetryRuntimeError::exporter("log", error))
        }
        OtlpTransport::Grpc => {
            let builder = opentelemetry_otlp::LogExporter::builder()
                .with_tonic()
                .with_endpoint(settings.endpoint.clone())
                .with_protocol(Protocol::Grpc)
                .with_timeout(settings.export_timeout)
                .with_interceptor(grpc_metadata_interceptor(&settings.headers)?);
            let result = match settings.compression {
                OtlpCompression::None => builder.build(),
                OtlpCompression::Gzip => builder.with_compression(Compression::Gzip).build(),
            };
            result.map_err(|error| TelemetryRuntimeError::exporter("log", error))
        }
    }
}

fn blocking_http_client(
    settings: &ValidatedTelemetrySettings,
) -> Result<RestrictedHttpClient, TelemetryRuntimeError> {
    let timeout = settings.export_timeout;
    let inner = std::thread::spawn(move || {
        reqwest::blocking::Client::builder()
            .use_rustls_tls()
            .timeout(timeout)
            .build()
    })
    .join()
    .map_err(|_| TelemetryRuntimeError::InvalidConfig("HTTP client setup panicked"))?
    .map_err(|error| TelemetryRuntimeError::exporter("http_client", error))?;
    let configured_headers = settings
        .headers
        .iter()
        .map(|(name, value)| {
            let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| TelemetryRuntimeError::Secret("header name is invalid for HTTP"))?;
            let value = reqwest::header::HeaderValue::from_str(value)
                .map_err(|_| TelemetryRuntimeError::Secret("header value is invalid for HTTP"))?;
            Ok((name, value))
        })
        .collect::<Result<Vec<_>, TelemetryRuntimeError>>()?;
    Ok(RestrictedHttpClient {
        inner,
        configured_headers,
    })
}

fn grpc_metadata_interceptor(
    headers: &HashMap<String, String>,
) -> Result<
    impl FnMut(tonic::Request<()>) -> Result<tonic::Request<()>, tonic::Status> + Clone,
    TelemetryRuntimeError,
> {
    let mut metadata = tonic::metadata::MetadataMap::with_capacity(headers.len());
    for (name, value) in headers {
        let name: tonic::metadata::MetadataKey<tonic::metadata::Ascii> = name
            .parse()
            .map_err(|_| TelemetryRuntimeError::Secret("header name is invalid for gRPC"))?;
        let value: tonic::metadata::MetadataValue<tonic::metadata::Ascii> = value
            .parse()
            .map_err(|_| TelemetryRuntimeError::Secret("header value is invalid for gRPC"))?;
        metadata.insert(name, value);
    }
    Ok(move |mut request: tonic::Request<()>| {
        *request.metadata_mut() = metadata.clone();
        Ok(request)
    })
}

fn http_signal_endpoint(settings: &ValidatedTelemetrySettings, signal_path: &str) -> String {
    format!("{}{}", settings.endpoint.trim_end_matches('/'), signal_path)
}

fn install_rustls_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

fn span_data(record: SpanRecord, scope: InstrumentationScope) -> SpanData {
    let context = span_context(record.context());
    let parent_span_id = record
        .parent_span_id()
        .map(SpanId::from_bytes)
        .unwrap_or(SpanId::INVALID);
    let mut links = SpanLinks::default();
    links.links = record
        .links()
        .iter()
        .copied()
        .map(span_context)
        .map(Link::with_context)
        .collect();

    SpanData {
        span_context: context,
        parent_span_id,
        parent_span_is_remote: false,
        span_kind: SpanKind::Internal,
        name: Cow::Borrowed(record.name()),
        start_time: system_time(record.started_unix_nanos()),
        end_time: system_time(record.ended_unix_nanos()),
        attributes: otel_attributes(record.attributes(), Some(record.descriptor_version())),
        dropped_attributes_count: 0,
        events: SpanEvents::default(),
        links,
        status: match record.status() {
            BitFunSpanStatus::Ok => Status::Ok,
            BitFunSpanStatus::Error => Status::error("operation failed"),
            BitFunSpanStatus::Unset => Status::Unset,
        },
        instrumentation_scope: scope,
    }
}

fn emit_log(logger: &SdkLogger, record: BitFunLogRecord) {
    let mut otel_record = logger.create_log_record();
    otel_record.set_event_name(record.event_name());
    otel_record.set_target("bitfun-observability");
    otel_record.set_timestamp(system_time(record.timestamp_unix_nanos()));
    otel_record.set_observed_timestamp(system_time(record.observed_unix_nanos()));
    let (severity, severity_text) = match record.severity() {
        BitFunSeverity::Debug => (Severity::Debug, "DEBUG"),
        BitFunSeverity::Info => (Severity::Info, "INFO"),
        BitFunSeverity::Warn => (Severity::Warn, "WARN"),
        BitFunSeverity::Error => (Severity::Error, "ERROR"),
    };
    otel_record.set_severity_number(severity);
    otel_record.set_severity_text(severity_text);
    otel_record.set_body(AnyValue::String(record.body().into()));
    otel_record.add_attributes(log_attributes(
        record.attributes(),
        Some(record.descriptor_version()),
    ));
    if let Some(context) = record.span_context() {
        otel_record.set_trace_context(
            TraceId::from_bytes(context.trace_id()),
            SpanId::from_bytes(context.span_id()),
            Some(if context.is_sampled() {
                TraceFlags::SAMPLED
            } else {
                TraceFlags::NOT_SAMPLED
            }),
        );
    }
    logger.emit(otel_record);
}

fn span_context(context: BitFunSpanContext) -> SpanContext {
    SpanContext::new(
        TraceId::from_bytes(context.trace_id()),
        SpanId::from_bytes(context.span_id()),
        if context.is_sampled() {
            TraceFlags::SAMPLED
        } else {
            TraceFlags::NOT_SAMPLED
        },
        false,
        TraceState::default(),
    )
}

fn otel_attributes(attributes: &[Attribute], schema_version: Option<u16>) -> Vec<KeyValue> {
    let mut output = Vec::with_capacity(attributes.len() + usize::from(schema_version.is_some()));
    if let Some(version) = schema_version {
        output.push(KeyValue::new(
            "bitfun.telemetry.schema.version",
            i64::from(version),
        ));
    }
    output.extend(
        attributes
            .iter()
            .map(|attribute| KeyValue::new(attribute.key(), otel_value(attribute.value()))),
    );
    output
}

fn otel_value(value: &AttributeValue) -> Value {
    match value {
        AttributeValue::Enum(value) | AttributeValue::Version(value) => {
            Value::String(value.clone().into())
        }
        AttributeValue::Bool(value) => Value::Bool(*value),
        AttributeValue::U64(value) => i64::try_from(*value)
            .map(Value::I64)
            .unwrap_or_else(|_| Value::String(value.to_string().into())),
        AttributeValue::I64(value) => Value::I64(*value),
        AttributeValue::F64(value) => Value::F64(*value),
    }
}

fn log_attributes(
    attributes: &[Attribute],
    schema_version: Option<u16>,
) -> Vec<(opentelemetry::Key, AnyValue)> {
    let mut output = Vec::with_capacity(attributes.len() + usize::from(schema_version.is_some()));
    if let Some(version) = schema_version {
        output.push((
            opentelemetry::Key::from_static_str("bitfun.telemetry.schema.version"),
            AnyValue::Int(i64::from(version)),
        ));
    }
    output.extend(attributes.iter().map(|attribute| {
        let value = match attribute.value() {
            AttributeValue::Enum(value) | AttributeValue::Version(value) => {
                AnyValue::String(value.clone().into())
            }
            AttributeValue::Bool(value) => AnyValue::Boolean(*value),
            AttributeValue::U64(value) => i64::try_from(*value)
                .map(AnyValue::Int)
                .unwrap_or_else(|_| AnyValue::String(value.to_string().into())),
            AttributeValue::I64(value) => AnyValue::Int(*value),
            AttributeValue::F64(value) => AnyValue::Double(*value),
        };
        (opentelemetry::Key::from_static_str(attribute.key()), value)
    }));
    output
}

fn metric_unit(name: &'static str) -> &'static str {
    match descriptor_registry()
        .iter()
        .find(|descriptor| descriptor.name() == name)
        .and_then(|descriptor| descriptor.metric_unit())
    {
        Some(MetricUnit::Seconds) => "s",
        Some(MetricUnit::Tokens) => "{token}",
        Some(MetricUnit::One) | None => "1",
    }
}

fn system_time(unix_nanos: u128) -> SystemTime {
    let seconds = (unix_nanos / 1_000_000_000).min(u128::from(u64::MAX)) as u64;
    let nanos = (unix_nanos % 1_000_000_000) as u32;
    UNIX_EPOCH
        .checked_add(Duration::new(seconds, nanos))
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::service::Interceptor;

    #[test]
    fn http_headers_are_rebuilt_from_transport_and_product_configuration() {
        let mut request = Request::builder()
            .header("content-type", "application/x-protobuf")
            .header("authorization", "environment-value")
            .header("x-injected", "environment-value")
            .body(Bytes::new())
            .unwrap();
        let configured = vec![(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_static("product-value"),
        )];

        restrict_http_headers(&mut request, &configured);

        assert_eq!(
            request.headers().get("content-type").unwrap(),
            "application/x-protobuf"
        );
        assert_eq!(
            request.headers().get("authorization").unwrap(),
            "product-value"
        );
        assert!(!request.headers().contains_key("x-injected"));
    }

    #[test]
    fn grpc_metadata_is_rebuilt_from_product_configuration() {
        let headers = HashMap::from([("authorization".to_string(), "product-value".to_string())]);
        let mut interceptor = grpc_metadata_interceptor(&headers).unwrap();
        let mut request = tonic::Request::new(());
        request
            .metadata_mut()
            .insert("x-injected", "environment-value".parse().unwrap());

        let request = interceptor.call(request).unwrap();

        assert_eq!(
            request.metadata().get("authorization").unwrap(),
            "product-value"
        );
        assert!(request.metadata().get("x-injected").is_none());
    }
}
