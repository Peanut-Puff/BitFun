use crate::{PolicySnapshot, SignalPolicy, TelemetryLevel};
use serde::{Deserialize, Deserializer, Serialize};

pub const TELEMETRY_CONFIG_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TelemetryConfig {
    pub version: u16,
    pub level: TelemetryLevel,
    pub signals: TelemetrySignalConfig,
    pub exporter: TelemetryExporterConfig,
    pub batch: TelemetryBatchConfig,
    pub sampling: TelemetrySamplingConfig,
}

impl TelemetryConfig {
    pub fn policy_snapshot(&self) -> PolicySnapshot {
        let success_log_ratio = match self.level {
            TelemetryLevel::Off => 0.0,
            TelemetryLevel::Basic => self.sampling.basic_success_log_ratio,
            TelemetryLevel::Diagnostic => self.sampling.diagnostic_success_log_ratio,
        };

        PolicySnapshot::new(self.level)
            .with_signals(SignalPolicy::new(
                self.signals.traces,
                self.signals.metrics,
                self.signals.logs,
            ))
            .with_trace_sample_ratio(self.sampling.diagnostic_trace_ratio)
            .with_success_log_sample_ratio(success_log_ratio)
    }

    pub fn is_enabled(&self) -> bool {
        self.level != TelemetryLevel::Off
    }
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            version: TELEMETRY_CONFIG_VERSION,
            level: TelemetryLevel::Off,
            signals: TelemetrySignalConfig::default(),
            exporter: TelemetryExporterConfig::default(),
            batch: TelemetryBatchConfig::default(),
            sampling: TelemetrySamplingConfig::default(),
        }
    }
}

impl<'de> Deserialize<'de> for TelemetryConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Representation {
            Legacy(bool),
            Current(CurrentTelemetryConfig),
        }

        match Representation::deserialize(deserializer)? {
            Representation::Legacy(enabled) => {
                let mut config = TelemetryConfig::default();
                if enabled {
                    config.level = TelemetryLevel::Basic;
                }
                Ok(config)
            }
            Representation::Current(current) => Ok(current.into()),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CurrentTelemetryConfig {
    version: Option<u16>,
    level: TelemetryLevel,
    signals: TelemetrySignalConfig,
    exporter: TelemetryExporterConfig,
    batch: TelemetryBatchConfig,
    sampling: TelemetrySamplingConfig,
}

impl From<CurrentTelemetryConfig> for TelemetryConfig {
    fn from(current: CurrentTelemetryConfig) -> Self {
        Self {
            version: current.version.unwrap_or(TELEMETRY_CONFIG_VERSION),
            level: current.level,
            signals: current.signals,
            exporter: current.exporter,
            batch: current.batch,
            sampling: current.sampling,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TelemetrySignalConfig {
    pub traces: bool,
    pub metrics: bool,
    pub logs: bool,
}

impl Default for TelemetrySignalConfig {
    fn default() -> Self {
        Self {
            traces: true,
            metrics: true,
            logs: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OtlpTransport {
    #[default]
    HttpProtobuf,
    Grpc,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OtlpCompression {
    None,
    #[default]
    Gzip,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TelemetryExporterConfig {
    pub endpoint: Option<String>,
    pub transport: OtlpTransport,
    pub compression: OtlpCompression,
    pub headers_secret_ref: Option<String>,
    pub allow_insecure_loopback: bool,
}

impl Default for TelemetryExporterConfig {
    fn default() -> Self {
        Self {
            endpoint: None,
            transport: OtlpTransport::HttpProtobuf,
            compression: OtlpCompression::Gzip,
            headers_secret_ref: None,
            allow_insecure_loopback: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TelemetryBatchConfig {
    pub max_queue_size: usize,
    pub max_export_batch_size: usize,
    pub scheduled_delay_ms: u64,
    pub metrics_export_interval_ms: u64,
    pub export_timeout_ms: u64,
    pub shutdown_timeout_ms: u64,
}

impl Default for TelemetryBatchConfig {
    fn default() -> Self {
        Self {
            max_queue_size: 2_048,
            max_export_batch_size: 512,
            scheduled_delay_ms: 5_000,
            metrics_export_interval_ms: 60_000,
            export_timeout_ms: 10_000,
            shutdown_timeout_ms: 3_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TelemetrySamplingConfig {
    pub diagnostic_trace_ratio: f64,
    pub basic_success_log_ratio: f64,
    pub diagnostic_success_log_ratio: f64,
}

impl Default for TelemetrySamplingConfig {
    fn default() -> Self {
        Self {
            diagnostic_trace_ratio: 0.1,
            basic_success_log_ratio: 0.1,
            diagnostic_success_log_ratio: 0.5,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_boolean_migrates_without_inventing_an_endpoint() {
        let enabled: TelemetryConfig = serde_json::from_str("true").unwrap();
        assert_eq!(enabled.level, TelemetryLevel::Basic);
        assert_eq!(enabled.exporter.endpoint, None);

        let disabled: TelemetryConfig = serde_json::from_str("false").unwrap();
        assert_eq!(disabled, TelemetryConfig::default());
    }

    #[test]
    fn missing_current_fields_receive_versioned_defaults() {
        let config: TelemetryConfig =
            serde_json::from_str(r#"{"level":"diagnostic","signals":{"logs":false}}"#).unwrap();

        assert_eq!(config.version, TELEMETRY_CONFIG_VERSION);
        assert_eq!(config.level, TelemetryLevel::Diagnostic);
        assert!(config.signals.traces);
        assert!(config.signals.metrics);
        assert!(!config.signals.logs);
        assert_eq!(config.batch, TelemetryBatchConfig::default());
    }

    #[test]
    fn serialization_always_uses_the_current_object_shape() {
        let serialized = serde_json::to_value(TelemetryConfig::default()).unwrap();
        assert_eq!(serialized["version"], TELEMETRY_CONFIG_VERSION);
        assert_eq!(serialized["level"], "off");
        assert!(serialized["exporter"].is_object());
    }
}
