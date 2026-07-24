//! Portable observability contracts for BitFun.
//!
//! Business owners submit typed lifecycle facts through [`domains`]. Records
//! reach a sink only after the schema and privacy gate accept them. This crate
//! deliberately has no dependency on an OpenTelemetry SDK, product assembly,
//! application entrypoints, or external transports.

mod facade;
mod model;
mod schema;
mod sink;

pub mod config;
pub mod domains;

pub use facade::{
    PolicySnapshot, SignalPolicy, Telemetry, TelemetryControl, TelemetryDiagnostics, TelemetrySpan,
};
pub use model::{
    Attribute, AttributeValue, LogRecord, MetricRecord, MetricValue, Severity, SignalKind,
    SpanContext, SpanRecord, SpanStatus, TelemetryLevel, ValidatedRecord,
};
pub use schema::{
    descriptor_registry, CardinalityBudget, DescriptorView, FieldType, FieldView, MetricUnit,
    PrivacyError, RateLimitClass, RetentionClass, SamplingClass,
};
pub use sink::{InMemorySink, NoopSink, TelemetrySink};
