use crate::{OtlpHeaders, TelemetryRuntimeError, TelemetrySecretProvider};
use bitfun_observability::config::{OtlpCompression, TelemetryConfig, TELEMETRY_CONFIG_VERSION};
use bitfun_observability::TelemetryLevel;
use std::path::PathBuf;
use std::time::Duration;
use url::Url;

const MAX_ENDPOINT_LENGTH: usize = 2_048;
const MAX_HEADER_VALUE_LENGTH: usize = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelemetryEntrypoint {
    Desktop,
    Cli,
    Server,
    Relay,
}

impl TelemetryEntrypoint {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Cli => "cli",
            Self::Server => "server",
            Self::Relay => "relay",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeploymentEnvironment {
    Development,
    Production,
    Test,
}

impl DeploymentEnvironment {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Development => "development",
            Self::Production => "production",
            Self::Test => "test",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TelemetryRuntimeMetadata {
    pub service_name: &'static str,
    pub service_version: &'static str,
    pub entrypoint: TelemetryEntrypoint,
    pub environment: DeploymentEnvironment,
    pub state_directory: PathBuf,
}

impl TelemetryRuntimeMetadata {
    pub fn new(
        service_name: &'static str,
        service_version: &'static str,
        entrypoint: TelemetryEntrypoint,
        state_directory: impl Into<PathBuf>,
    ) -> Self {
        Self {
            service_name,
            service_version,
            entrypoint,
            environment: if cfg!(debug_assertions) {
                DeploymentEnvironment::Development
            } else {
                DeploymentEnvironment::Production
            },
            state_directory: state_directory.into(),
        }
    }

    pub fn with_environment(mut self, environment: DeploymentEnvironment) -> Self {
        self.environment = environment;
        self
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ValidatedTelemetrySettings {
    pub endpoint: String,
    pub audience: String,
    pub compression: OtlpCompression,
    pub headers: OtlpHeaders,
    pub max_queue_size: usize,
    pub max_export_batch_size: usize,
    pub scheduled_delay: Duration,
    pub metrics_export_interval: Duration,
    pub export_timeout: Duration,
    pub shutdown_timeout: Duration,
}

pub(crate) fn validate_enabled_config(
    config: &TelemetryConfig,
    secrets: &dyn TelemetrySecretProvider,
) -> Result<ValidatedTelemetrySettings, TelemetryRuntimeError> {
    if config.version != TELEMETRY_CONFIG_VERSION {
        return Err(TelemetryRuntimeError::InvalidConfig(
            "unsupported config version",
        ));
    }
    if config.level == TelemetryLevel::Off {
        return Err(TelemetryRuntimeError::InvalidConfig(
            "enabled runtime requires a non-off level",
        ));
    }
    if !config.signals.traces && !config.signals.metrics && !config.signals.logs {
        return Err(TelemetryRuntimeError::InvalidConfig(
            "at least one telemetry signal must be enabled",
        ));
    }
    validate_ratio(config.sampling.diagnostic_trace_ratio)?;
    validate_ratio(config.sampling.basic_success_log_ratio)?;
    validate_ratio(config.sampling.diagnostic_success_log_ratio)?;

    if config.batch.max_queue_size == 0 || config.batch.max_queue_size > 65_536 {
        return Err(TelemetryRuntimeError::InvalidConfig(
            "max_queue_size must be between 1 and 65536",
        ));
    }
    if config.batch.max_export_batch_size == 0
        || config.batch.max_export_batch_size > config.batch.max_queue_size
    {
        return Err(TelemetryRuntimeError::InvalidConfig(
            "max_export_batch_size must fit within the queue",
        ));
    }
    for (value, field) in [
        (config.batch.scheduled_delay_ms, "scheduled_delay_ms"),
        (
            config.batch.metrics_export_interval_ms,
            "metrics_export_interval_ms",
        ),
        (config.batch.export_timeout_ms, "export_timeout_ms"),
        (config.batch.shutdown_timeout_ms, "shutdown_timeout_ms"),
    ] {
        if !(10..=300_000).contains(&value) {
            return Err(TelemetryRuntimeError::InvalidConfig(field));
        }
    }

    let endpoint = config
        .exporter
        .endpoint
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(TelemetryRuntimeError::InvalidConfig(
            "endpoint is required when telemetry is enabled",
        ))?;
    if endpoint.len() > MAX_ENDPOINT_LENGTH {
        return Err(TelemetryRuntimeError::InvalidConfig("endpoint is too long"));
    }
    let parsed = Url::parse(endpoint)
        .map_err(|_| TelemetryRuntimeError::InvalidConfig("endpoint is not a valid URL"))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(TelemetryRuntimeError::InvalidConfig(
            "endpoint must be an HTTP or HTTPS URL with a host",
        ));
    }
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(TelemetryRuntimeError::InvalidConfig(
            "endpoint must not contain credentials, query, or fragment",
        ));
    }
    if parsed.scheme() != "https"
        && !(config.exporter.allow_insecure_loopback && is_loopback(&parsed))
    {
        return Err(TelemetryRuntimeError::InvalidConfig(
            "endpoint must use HTTPS; HTTP is limited to explicitly enabled loopback collectors",
        ));
    }

    let headers = match config.exporter.headers_secret_ref.as_deref() {
        Some(reference) => {
            if !valid_secret_reference(reference) {
                return Err(TelemetryRuntimeError::InvalidConfig(
                    "headers_secret_ref has an invalid shape",
                ));
            }
            validate_headers(secrets.resolve_headers(reference)?)?
        }
        None => OtlpHeaders::new(),
    };
    let audience = format!(
        "{}|{}",
        parsed.as_str().trim_end_matches('/'),
        config
            .exporter
            .headers_secret_ref
            .as_deref()
            .unwrap_or("no-credential")
    );

    Ok(ValidatedTelemetrySettings {
        endpoint: endpoint.trim_end_matches('/').to_string(),
        audience,
        compression: config.exporter.compression,
        headers,
        max_queue_size: config.batch.max_queue_size,
        max_export_batch_size: config.batch.max_export_batch_size,
        scheduled_delay: Duration::from_millis(config.batch.scheduled_delay_ms),
        metrics_export_interval: Duration::from_millis(config.batch.metrics_export_interval_ms),
        export_timeout: Duration::from_millis(config.batch.export_timeout_ms),
        shutdown_timeout: Duration::from_millis(config.batch.shutdown_timeout_ms),
    })
}

pub fn deployment_config_from_env(prefix: &str) -> Result<TelemetryConfig, TelemetryRuntimeError> {
    deployment_config_from_source(prefix, |name| std::env::var(name).ok())
}

fn deployment_config_from_source(
    prefix: &str,
    mut source: impl FnMut(&str) -> Option<String>,
) -> Result<TelemetryConfig, TelemetryRuntimeError> {
    if prefix.is_empty()
        || !prefix
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(TelemetryRuntimeError::InvalidConfig(
            "deployment environment prefix is invalid",
        ));
    }

    let mut config = TelemetryConfig::default();
    if let Some(value) = source_value(prefix, "LEVEL", &mut source) {
        config.level = match value.as_str() {
            "off" => TelemetryLevel::Off,
            "basic" => TelemetryLevel::Basic,
            "diagnostic" => TelemetryLevel::Diagnostic,
            _ => return Err(TelemetryRuntimeError::InvalidConfig("LEVEL is invalid")),
        };
    }
    if let Some(value) = source_value(prefix, "ENDPOINT", &mut source) {
        config.exporter.endpoint = Some(value);
    }
    if let Some(value) = source_value(prefix, "COMPRESSION", &mut source) {
        config.exporter.compression = match value.as_str() {
            "none" => OtlpCompression::None,
            "gzip" => OtlpCompression::Gzip,
            _ => {
                return Err(TelemetryRuntimeError::InvalidConfig(
                    "COMPRESSION is invalid",
                ));
            }
        };
    }
    if let Some(value) = source_value(prefix, "HEADERS_SECRET_REF", &mut source) {
        config.exporter.headers_secret_ref = Some(value);
    }
    if let Some(value) = source_value(prefix, "ALLOW_INSECURE_LOOPBACK", &mut source) {
        config.exporter.allow_insecure_loopback = parse_bool(&value)?;
    }
    for (suffix, target) in [
        ("TRACES_ENABLED", &mut config.signals.traces),
        ("METRICS_ENABLED", &mut config.signals.metrics),
        ("LOGS_ENABLED", &mut config.signals.logs),
    ] {
        if let Some(value) = source_value(prefix, suffix, &mut source) {
            *target = parse_bool(&value)?;
        }
    }
    for (suffix, target, field) in [
        (
            "MAX_QUEUE_SIZE",
            &mut config.batch.max_queue_size,
            "MAX_QUEUE_SIZE is invalid",
        ),
        (
            "MAX_EXPORT_BATCH_SIZE",
            &mut config.batch.max_export_batch_size,
            "MAX_EXPORT_BATCH_SIZE is invalid",
        ),
    ] {
        if let Some(value) = source_value(prefix, suffix, &mut source) {
            *target = parse_usize(&value, field)?;
        }
    }
    for (suffix, target, field) in [
        (
            "SCHEDULED_DELAY_MS",
            &mut config.batch.scheduled_delay_ms,
            "SCHEDULED_DELAY_MS is invalid",
        ),
        (
            "METRICS_EXPORT_INTERVAL_MS",
            &mut config.batch.metrics_export_interval_ms,
            "METRICS_EXPORT_INTERVAL_MS is invalid",
        ),
        (
            "EXPORT_TIMEOUT_MS",
            &mut config.batch.export_timeout_ms,
            "EXPORT_TIMEOUT_MS is invalid",
        ),
        (
            "SHUTDOWN_TIMEOUT_MS",
            &mut config.batch.shutdown_timeout_ms,
            "SHUTDOWN_TIMEOUT_MS is invalid",
        ),
    ] {
        if let Some(value) = source_value(prefix, suffix, &mut source) {
            *target = parse_u64(&value, field)?;
        }
    }
    for (suffix, target, field) in [
        (
            "DIAGNOSTIC_TRACE_RATIO",
            &mut config.sampling.diagnostic_trace_ratio,
            "DIAGNOSTIC_TRACE_RATIO is invalid",
        ),
        (
            "BASIC_SUCCESS_LOG_RATIO",
            &mut config.sampling.basic_success_log_ratio,
            "BASIC_SUCCESS_LOG_RATIO is invalid",
        ),
        (
            "DIAGNOSTIC_SUCCESS_LOG_RATIO",
            &mut config.sampling.diagnostic_success_log_ratio,
            "DIAGNOSTIC_SUCCESS_LOG_RATIO is invalid",
        ),
    ] {
        if let Some(value) = source_value(prefix, suffix, &mut source) {
            *target = parse_f64(&value, field)?;
        }
    }
    Ok(config)
}

fn source_value(
    prefix: &str,
    suffix: &str,
    source: &mut impl FnMut(&str) -> Option<String>,
) -> Option<String> {
    source(&format!("{prefix}_{suffix}"))
}

fn parse_bool(value: &str) -> Result<bool, TelemetryRuntimeError> {
    match value {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => Err(TelemetryRuntimeError::InvalidConfig(
            "boolean deployment setting is invalid",
        )),
    }
}

fn parse_usize(value: &str, error: &'static str) -> Result<usize, TelemetryRuntimeError> {
    value
        .parse()
        .map_err(|_| TelemetryRuntimeError::InvalidConfig(error))
}

fn parse_u64(value: &str, error: &'static str) -> Result<u64, TelemetryRuntimeError> {
    value
        .parse()
        .map_err(|_| TelemetryRuntimeError::InvalidConfig(error))
}

fn parse_f64(value: &str, error: &'static str) -> Result<f64, TelemetryRuntimeError> {
    value
        .parse()
        .map_err(|_| TelemetryRuntimeError::InvalidConfig(error))
}

fn validate_ratio(value: f64) -> Result<(), TelemetryRuntimeError> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(TelemetryRuntimeError::InvalidConfig(
            "sampling ratios must be between 0 and 1",
        ))
    }
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    }
}

fn valid_secret_reference(reference: &str) -> bool {
    !reference.is_empty()
        && reference.len() <= 160
        && reference.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':' | b'/')
        })
}

fn validate_headers(headers: OtlpHeaders) -> Result<OtlpHeaders, TelemetryRuntimeError> {
    if headers.len() > 32 {
        return Err(TelemetryRuntimeError::Secret(
            "secret contains too many headers",
        ));
    }
    headers
        .into_iter()
        .map(|(name, value)| {
            let name = name.to_ascii_lowercase();
            if name.is_empty()
                || name.len() > 128
                || name.ends_with("-bin")
                || reserved_transport_header(&name)
                || !name.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'-' | b'_' | b'.')
                })
            {
                return Err(TelemetryRuntimeError::Secret(
                    "secret contains an invalid header name",
                ));
            }
            if value.len() > MAX_HEADER_VALUE_LENGTH
                || value.bytes().any(|byte| matches!(byte, b'\r' | b'\n' | 0))
            {
                return Err(TelemetryRuntimeError::Secret(
                    "secret contains an invalid header value",
                ));
            }
            Ok((name, value))
        })
        .collect()
}

fn reserved_transport_header(name: &str) -> bool {
    matches!(
        name,
        "host"
            | "connection"
            | "content-type"
            | "content-length"
            | "content-encoding"
            | "transfer-encoding"
            | "user-agent"
            | "traceparent"
            | "tracestate"
            | "baggage"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NoTelemetrySecrets;

    fn local_config() -> TelemetryConfig {
        TelemetryConfig {
            level: TelemetryLevel::Diagnostic,
            exporter: bitfun_observability::config::TelemetryExporterConfig {
                endpoint: Some("http://127.0.0.1:4318".to_string()),
                allow_insecure_loopback: true,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn local_http_requires_an_explicit_loopback_exception() {
        let mut config = local_config();
        config.exporter.allow_insecure_loopback = false;
        assert!(validate_enabled_config(&config, &NoTelemetrySecrets).is_err());

        config.exporter.allow_insecure_loopback = true;
        assert!(validate_enabled_config(&config, &NoTelemetrySecrets).is_ok());

        config.exporter.endpoint = Some("http://collector.example.test:4318".to_string());
        assert!(validate_enabled_config(&config, &NoTelemetrySecrets).is_err());
    }

    #[test]
    fn endpoint_rejects_embedded_credentials() {
        let mut config = local_config();
        config.exporter.endpoint = Some("http://user:pass@127.0.0.1:4318".to_string());
        assert!(validate_enabled_config(&config, &NoTelemetrySecrets).is_err());
    }

    #[test]
    fn anonymous_identity_audience_includes_the_full_collector_base_path() {
        let mut first = local_config();
        first.exporter.endpoint = Some("http://127.0.0.1:4318/tenant-a".to_string());
        let mut second = first.clone();
        second.exporter.endpoint = Some("http://127.0.0.1:4318/tenant-b".to_string());

        let first = validate_enabled_config(&first, &NoTelemetrySecrets).unwrap();
        let second = validate_enabled_config(&second, &NoTelemetrySecrets).unwrap();

        assert_ne!(first.audience, second.audience);
    }

    #[test]
    fn invalid_batch_configuration_is_rejected_before_identity_creation() {
        let mut config = local_config();
        config.batch.max_export_batch_size = config.batch.max_queue_size + 1;
        assert!(validate_enabled_config(&config, &NoTelemetrySecrets).is_err());
    }

    #[test]
    fn enabled_config_requires_at_least_one_signal() {
        let mut config = local_config();
        config.signals.traces = false;
        config.signals.metrics = false;
        config.signals.logs = false;

        assert!(validate_enabled_config(&config, &NoTelemetrySecrets).is_err());
    }

    #[test]
    fn credentials_cannot_override_transport_or_trace_headers() {
        for name in [
            "host",
            "content-type",
            "traceparent",
            "tracestate",
            "baggage",
        ] {
            assert!(reserved_transport_header(name), "{name}");
        }
        assert!(!reserved_transport_header("authorization"));
        assert!(!reserved_transport_header("x-api-key"));
        assert!(validate_headers(OtlpHeaders::from([(
            "traceparent".to_string(),
            "forged".to_string(),
        )]))
        .is_err());
        assert!(validate_headers(OtlpHeaders::from([(
            "authorization".to_string(),
            "redacted-test-value".to_string(),
        )]))
        .is_ok());
    }

    #[test]
    fn deployment_number_parsers_reject_malformed_values() {
        assert_eq!(parse_usize("32", "invalid").unwrap(), 32);
        assert_eq!(parse_u64("2000", "invalid").unwrap(), 2_000);
        assert_eq!(parse_f64("0.25", "invalid").unwrap(), 0.25);
        assert!(parse_usize("-1", "invalid").is_err());
        assert!(parse_u64("1.5", "invalid").is_err());
        assert!(parse_f64("ratio", "invalid").is_err());
    }

    #[test]
    fn deployment_source_maps_the_complete_runtime_configuration() {
        let values = std::collections::HashMap::from([
            ("TEST_LEVEL", "diagnostic"),
            ("TEST_ENDPOINT", "http://127.0.0.1:4318"),
            ("TEST_COMPRESSION", "none"),
            ("TEST_HEADERS_SECRET_REF", "env:TEST_HEADERS"),
            ("TEST_ALLOW_INSECURE_LOOPBACK", "true"),
            ("TEST_TRACES_ENABLED", "false"),
            ("TEST_METRICS_ENABLED", "true"),
            ("TEST_LOGS_ENABLED", "true"),
            ("TEST_MAX_QUEUE_SIZE", "128"),
            ("TEST_MAX_EXPORT_BATCH_SIZE", "32"),
            ("TEST_SCHEDULED_DELAY_MS", "250"),
            ("TEST_METRICS_EXPORT_INTERVAL_MS", "1000"),
            ("TEST_EXPORT_TIMEOUT_MS", "2000"),
            ("TEST_SHUTDOWN_TIMEOUT_MS", "1500"),
            ("TEST_DIAGNOSTIC_TRACE_RATIO", "0.25"),
            ("TEST_BASIC_SUCCESS_LOG_RATIO", "0.1"),
            ("TEST_DIAGNOSTIC_SUCCESS_LOG_RATIO", "0.5"),
        ]);

        let config = deployment_config_from_source("TEST", |name| {
            values.get(name).map(|value| (*value).to_string())
        })
        .unwrap();

        assert_eq!(config.level, TelemetryLevel::Diagnostic);
        assert_eq!(config.exporter.compression, OtlpCompression::None);
        assert_eq!(
            config.exporter.headers_secret_ref.as_deref(),
            Some("env:TEST_HEADERS")
        );
        assert!(!config.signals.traces);
        assert!(config.signals.metrics);
        assert!(config.signals.logs);
        assert_eq!(config.batch.max_queue_size, 128);
        assert_eq!(config.batch.max_export_batch_size, 32);
        assert_eq!(config.batch.scheduled_delay_ms, 250);
        assert_eq!(config.batch.metrics_export_interval_ms, 1_000);
        assert_eq!(config.batch.export_timeout_ms, 2_000);
        assert_eq!(config.batch.shutdown_timeout_ms, 1_500);
        assert_eq!(config.sampling.diagnostic_trace_ratio, 0.25);
        assert_eq!(config.sampling.basic_success_log_ratio, 0.1);
        assert_eq!(config.sampling.diagnostic_success_log_ratio, 0.5);
    }
}
