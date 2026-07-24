use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryLevel {
    #[default]
    Off,
    Basic,
    Diagnostic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalKind {
    Trace,
    Metric,
    Log,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpanStatus {
    Ok,
    Error,
    Unset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SpanContext {
    trace_id: [u8; 16],
    span_id: [u8; 8],
    sampled: bool,
}

impl SpanContext {
    pub fn trace_id(&self) -> [u8; 16] {
        self.trace_id
    }

    pub fn span_id(&self) -> [u8; 8] {
        self.span_id
    }

    pub fn is_sampled(&self) -> bool {
        self.sampled
    }

    pub(crate) fn root(sample_ratio: f64) -> Self {
        let trace_id = *uuid::Uuid::new_v4().as_bytes();
        let span_id = next_span_id();
        let sample_key = u64::from_be_bytes(trace_id[..8].try_into().unwrap_or([0; 8]));
        let threshold = (sample_ratio.clamp(0.0, 1.0) * u64::MAX as f64) as u64;
        Self {
            trace_id,
            span_id,
            sampled: sample_key <= threshold,
        }
    }

    pub(crate) fn child(parent: Self) -> Self {
        Self {
            trace_id: parent.trace_id,
            span_id: next_span_id(),
            sampled: parent.sampled,
        }
    }
}

fn next_span_id() -> [u8; 8] {
    let bytes = uuid::Uuid::new_v4().into_bytes();
    bytes[..8].try_into().unwrap_or([1; 8])
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum AttributeValue {
    Enum(String),
    Bool(bool),
    U64(u64),
    I64(i64),
    F64(f64),
    Version(String),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Attribute {
    key: &'static str,
    value: AttributeValue,
}

impl Attribute {
    pub fn key(&self) -> &'static str {
        self.key
    }

    pub fn value(&self) -> &AttributeValue {
        &self.value
    }

    pub(crate) fn enumeration(key: &'static str, value: &'static str) -> Self {
        Self {
            key,
            value: AttributeValue::Enum(value.to_string()),
        }
    }

    pub(crate) fn boolean(key: &'static str, value: bool) -> Self {
        Self {
            key,
            value: AttributeValue::Bool(value),
        }
    }

    pub(crate) fn u64(key: &'static str, value: u64) -> Self {
        Self {
            key,
            value: AttributeValue::U64(value),
        }
    }

    pub(crate) fn f64(key: &'static str, value: f64) -> Self {
        Self {
            key,
            value: AttributeValue::F64(value),
        }
    }

    pub(crate) fn version(key: &'static str, value: impl Into<String>) -> Self {
        Self {
            key,
            value: AttributeValue::Version(value.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum MetricValue {
    Counter(u64),
    Histogram(f64),
    UpDownCounter(i64),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SpanRecord {
    pub(crate) descriptor_version: u16,
    pub(crate) name: &'static str,
    pub(crate) context: SpanContext,
    pub(crate) parent_span_id: Option<[u8; 8]>,
    pub(crate) links: Vec<SpanContext>,
    pub(crate) started_unix_nanos: u128,
    pub(crate) ended_unix_nanos: u128,
    pub(crate) status: SpanStatus,
    pub(crate) attributes: Vec<Attribute>,
}

impl SpanRecord {
    pub fn descriptor_version(&self) -> u16 {
        self.descriptor_version
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn context(&self) -> SpanContext {
        self.context
    }

    pub fn parent_span_id(&self) -> Option<[u8; 8]> {
        self.parent_span_id
    }

    pub fn links(&self) -> &[SpanContext] {
        &self.links
    }

    pub fn started_unix_nanos(&self) -> u128 {
        self.started_unix_nanos
    }

    pub fn ended_unix_nanos(&self) -> u128 {
        self.ended_unix_nanos
    }

    pub fn status(&self) -> SpanStatus {
        self.status
    }

    pub fn attributes(&self) -> &[Attribute] {
        &self.attributes
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MetricRecord {
    pub(crate) descriptor_version: u16,
    pub(crate) name: &'static str,
    pub(crate) timestamp_unix_nanos: u128,
    pub(crate) value: MetricValue,
    pub(crate) attributes: Vec<Attribute>,
}

impl MetricRecord {
    pub fn descriptor_version(&self) -> u16 {
        self.descriptor_version
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn timestamp_unix_nanos(&self) -> u128 {
        self.timestamp_unix_nanos
    }

    pub fn value(&self) -> &MetricValue {
        &self.value
    }

    pub fn attributes(&self) -> &[Attribute] {
        &self.attributes
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LogRecord {
    pub(crate) descriptor_version: u16,
    pub(crate) event_name: &'static str,
    pub(crate) timestamp_unix_nanos: u128,
    pub(crate) observed_unix_nanos: u128,
    pub(crate) severity: Severity,
    pub(crate) body: &'static str,
    pub(crate) span_context: Option<SpanContext>,
    pub(crate) attributes: Vec<Attribute>,
}

impl LogRecord {
    pub fn descriptor_version(&self) -> u16 {
        self.descriptor_version
    }

    pub fn event_name(&self) -> &'static str {
        self.event_name
    }

    pub fn timestamp_unix_nanos(&self) -> u128 {
        self.timestamp_unix_nanos
    }

    pub fn observed_unix_nanos(&self) -> u128 {
        self.observed_unix_nanos
    }

    pub fn severity(&self) -> Severity {
        self.severity
    }

    pub fn body(&self) -> &'static str {
        self.body
    }

    pub fn span_context(&self) -> Option<SpanContext> {
        self.span_context
    }

    pub fn attributes(&self) -> &[Attribute] {
        &self.attributes
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "signal", content = "record", rename_all = "snake_case")]
pub enum ValidatedRecord {
    Span(SpanRecord),
    Metric(MetricRecord),
    Log(LogRecord),
}

impl ValidatedRecord {
    pub fn signal_kind(&self) -> SignalKind {
        match self {
            Self::Span(_) => SignalKind::Trace,
            Self::Metric(_) => SignalKind::Metric,
            Self::Log(_) => SignalKind::Log,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Span(record) => record.name,
            Self::Metric(record) => record.name,
            Self::Log(record) => record.event_name,
        }
    }

    pub fn attributes(&self) -> &[Attribute] {
        match self {
            Self::Span(record) => &record.attributes,
            Self::Metric(record) => &record.attributes,
            Self::Log(record) => &record.attributes,
        }
    }
}
