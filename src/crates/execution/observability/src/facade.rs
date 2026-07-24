use crate::model::{
    Attribute, LogRecord, MetricRecord, MetricValue, Severity, SpanContext, SpanRecord, SpanStatus,
    TelemetryLevel, ValidatedRecord,
};
use crate::schema::{
    filter_attributes, operation_schema, token_descriptor, validate, OperationKind,
    OperationSchema, SamplingClass,
};
use crate::sink::{NoopSink, TelemetrySink};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignalPolicy {
    traces: bool,
    metrics: bool,
    logs: bool,
}

impl SignalPolicy {
    pub const fn all() -> Self {
        Self {
            traces: true,
            metrics: true,
            logs: true,
        }
    }

    pub const fn new(traces: bool, metrics: bool, logs: bool) -> Self {
        Self {
            traces,
            metrics,
            logs,
        }
    }

    pub fn traces(&self) -> bool {
        self.traces
    }

    pub fn metrics(&self) -> bool {
        self.metrics
    }

    pub fn logs(&self) -> bool {
        self.logs
    }
}

impl Default for SignalPolicy {
    fn default() -> Self {
        Self::all()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PolicySnapshot {
    level: TelemetryLevel,
    signals: SignalPolicy,
    trace_sample_ratio: f64,
    success_log_sample_ratio: f64,
}

impl PolicySnapshot {
    pub fn new(level: TelemetryLevel) -> Self {
        let (trace_sample_ratio, success_log_sample_ratio) = match level {
            TelemetryLevel::Off => (0.0, 0.0),
            TelemetryLevel::Basic => (0.0, 0.1),
            TelemetryLevel::Diagnostic => (0.1, 0.5),
        };
        Self {
            level,
            signals: SignalPolicy::all(),
            trace_sample_ratio,
            success_log_sample_ratio,
        }
    }

    pub fn with_signals(mut self, signals: SignalPolicy) -> Self {
        self.signals = signals;
        self
    }

    pub fn with_trace_sample_ratio(mut self, ratio: f64) -> Self {
        self.trace_sample_ratio = ratio.clamp(0.0, 1.0);
        self
    }

    pub fn with_success_log_sample_ratio(mut self, ratio: f64) -> Self {
        self.success_log_sample_ratio = ratio.clamp(0.0, 1.0);
        self
    }

    pub fn level(&self) -> TelemetryLevel {
        self.level
    }

    pub fn signals(&self) -> SignalPolicy {
        self.signals
    }

    pub fn trace_sample_ratio(&self) -> f64 {
        self.trace_sample_ratio
    }

    pub fn success_log_sample_ratio(&self) -> f64 {
        self.success_log_sample_ratio
    }
}

impl Default for PolicySnapshot {
    fn default() -> Self {
        Self::new(TelemetryLevel::Off)
    }
}

#[derive(Debug, Default)]
struct DiagnosticCounters {
    accepted: AtomicU64,
    rejected: AtomicU64,
    skipped: AtomicU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TelemetryDiagnostics {
    accepted: u64,
    rejected: u64,
    skipped: u64,
}

impl TelemetryDiagnostics {
    pub fn accepted(&self) -> u64 {
        self.accepted
    }

    pub fn rejected(&self) -> u64 {
        self.rejected
    }

    pub fn skipped(&self) -> u64 {
        self.skipped
    }
}

struct TelemetryInner {
    policy: RwLock<PolicySnapshot>,
    policy_revision: AtomicU64,
    sample_sequence: AtomicU64,
    sink: Arc<dyn TelemetrySink>,
    diagnostics: DiagnosticCounters,
}

#[derive(Clone)]
pub struct Telemetry {
    inner: Arc<TelemetryInner>,
}

impl std::fmt::Debug for Telemetry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Telemetry")
            .field("policy", &self.policy_snapshot())
            .field("diagnostics", &self.diagnostics())
            .finish_non_exhaustive()
    }
}

impl Telemetry {
    pub fn build(policy: PolicySnapshot, sink: Arc<dyn TelemetrySink>) -> (Self, TelemetryControl) {
        let telemetry = Self {
            inner: Arc::new(TelemetryInner {
                policy: RwLock::new(policy),
                policy_revision: AtomicU64::new(1),
                sample_sequence: AtomicU64::new(0),
                sink,
                diagnostics: DiagnosticCounters::default(),
            }),
        };
        let control = TelemetryControl {
            telemetry: telemetry.clone(),
        };
        (telemetry, control)
    }

    pub fn noop() -> Self {
        Self::build(PolicySnapshot::default(), Arc::new(NoopSink)).0
    }

    pub fn policy_snapshot(&self) -> PolicySnapshot {
        *self
            .inner
            .policy
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn diagnostics(&self) -> TelemetryDiagnostics {
        TelemetryDiagnostics {
            accepted: self.inner.diagnostics.accepted.load(Ordering::Relaxed),
            rejected: self.inner.diagnostics.rejected.load(Ordering::Relaxed),
            skipped: self.inner.diagnostics.skipped.load(Ordering::Relaxed),
        }
    }

    pub(crate) fn start_operation(
        &self,
        kind: OperationKind,
        attributes: Vec<Attribute>,
        parent: Option<SpanContext>,
        links: Vec<SpanContext>,
    ) -> TelemetrySpan {
        let policy = self.policy_snapshot();
        let revision = self.inner.policy_revision.load(Ordering::Acquire);
        if policy.level == TelemetryLevel::Off {
            return TelemetrySpan::disabled(self.clone(), operation_schema(kind), revision);
        }

        let context = if policy.level == TelemetryLevel::Diagnostic && policy.signals.traces {
            Some(match parent {
                Some(parent) => SpanContext::child(parent),
                None => SpanContext::root(policy.trace_sample_ratio),
            })
        } else {
            None
        };
        TelemetrySpan {
            telemetry: self.clone(),
            schema: operation_schema(kind),
            policy_revision: revision,
            started_at: Instant::now(),
            started_unix_nanos: unix_nanos(),
            context,
            parent_span_id: parent.map(|context| context.span_id()),
            links,
            attributes,
            active: true,
            closed: false,
        }
    }

    pub(crate) fn record_token_metric(&self, value: u64, attributes: Vec<Attribute>) {
        let revision = self.inner.policy_revision.load(Ordering::Acquire);
        let descriptor = token_descriptor();
        let record = ValidatedRecord::Metric(MetricRecord {
            descriptor_version: descriptor.version(),
            name: descriptor.name(),
            timestamp_unix_nanos: unix_nanos(),
            value: MetricValue::Histogram(value as f64),
            attributes,
        });
        self.emit_if_allowed(record, revision);
    }

    fn finish_span(
        &self,
        span: &mut TelemetrySpan,
        finish_attributes: Vec<Attribute>,
        status: SpanStatus,
        severity: Severity,
    ) {
        if !span.active || span.closed {
            return;
        }
        span.closed = true;

        if self.inner.policy_revision.load(Ordering::Acquire) != span.policy_revision {
            self.inner
                .diagnostics
                .skipped
                .fetch_add(1, Ordering::Relaxed);
            return;
        }

        let elapsed = span.started_at.elapsed();
        let duration_ms = elapsed.as_millis().min(u128::from(u64::MAX)) as u64;
        let mut attributes = std::mem::take(&mut span.attributes);
        attributes.extend(finish_attributes);
        attributes.push(Attribute::u64("bitfun.duration_ms", duration_ms));
        let ended_unix_nanos = unix_nanos();

        if let Some(context) = span.context.filter(|context| context.is_sampled()) {
            self.emit_if_allowed(
                ValidatedRecord::Span(SpanRecord {
                    descriptor_version: 1,
                    name: span.schema.span_name,
                    context,
                    parent_span_id: span.parent_span_id,
                    links: span.links.clone(),
                    started_unix_nanos: span.started_unix_nanos,
                    ended_unix_nanos,
                    status,
                    attributes: attributes.clone(),
                }),
                span.policy_revision,
            );
        }

        let metric_attributes = filter_attributes(&attributes, span.schema.metric_fields);
        self.emit_if_allowed(
            ValidatedRecord::Metric(MetricRecord {
                descriptor_version: 1,
                name: span.schema.total_metric_name,
                timestamp_unix_nanos: ended_unix_nanos,
                value: MetricValue::Counter(1),
                attributes: metric_attributes.clone(),
            }),
            span.policy_revision,
        );
        self.emit_if_allowed(
            ValidatedRecord::Metric(MetricRecord {
                descriptor_version: 1,
                name: span.schema.duration_metric_name,
                timestamp_unix_nanos: ended_unix_nanos,
                value: MetricValue::Histogram(elapsed.as_secs_f64()),
                attributes: metric_attributes,
            }),
            span.policy_revision,
        );

        let policy = self.policy_snapshot();
        if should_emit_log(
            policy,
            span.schema.sampling,
            severity,
            self.inner.sample_sequence.fetch_add(1, Ordering::Relaxed),
        ) {
            self.emit_if_allowed(
                ValidatedRecord::Log(LogRecord {
                    descriptor_version: 1,
                    event_name: span.schema.log_name,
                    timestamp_unix_nanos: ended_unix_nanos,
                    observed_unix_nanos: unix_nanos(),
                    severity,
                    body: span.schema.log_body,
                    span_context: span.context.filter(|context| context.is_sampled()),
                    attributes,
                }),
                span.policy_revision,
            );
        }
    }

    fn emit_if_allowed(&self, record: ValidatedRecord, revision: u64) {
        if self.inner.policy_revision.load(Ordering::Acquire) != revision {
            self.inner
                .diagnostics
                .skipped
                .fetch_add(1, Ordering::Relaxed);
            return;
        }
        let policy = self.policy_snapshot();
        let enabled = match record.signal_kind() {
            crate::SignalKind::Trace => {
                policy.level == TelemetryLevel::Diagnostic && policy.signals.traces
            }
            crate::SignalKind::Metric => {
                policy.level != TelemetryLevel::Off && policy.signals.metrics
            }
            crate::SignalKind::Log => policy.level != TelemetryLevel::Off && policy.signals.logs,
        };
        if !enabled {
            self.inner
                .diagnostics
                .skipped
                .fetch_add(1, Ordering::Relaxed);
            return;
        }
        if validate(&record).is_err() {
            self.inner
                .diagnostics
                .rejected
                .fetch_add(1, Ordering::Relaxed);
            return;
        }
        if self.inner.policy_revision.load(Ordering::Acquire) != revision {
            self.inner
                .diagnostics
                .skipped
                .fetch_add(1, Ordering::Relaxed);
            return;
        }
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.inner.sink.emit(record);
        }))
        .is_ok()
        {
            self.inner
                .diagnostics
                .accepted
                .fetch_add(1, Ordering::Relaxed);
        } else {
            self.inner
                .diagnostics
                .rejected
                .fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Control-plane capability retained by product assembly or the telemetry
/// runtime. Business instrumentation receives only [`Telemetry`].
#[derive(Clone)]
pub struct TelemetryControl {
    telemetry: Telemetry,
}

impl std::fmt::Debug for TelemetryControl {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TelemetryControl")
            .field("policy", &self.telemetry.policy_snapshot())
            .finish_non_exhaustive()
    }
}

impl TelemetryControl {
    /// Applies a new consent/signal policy.
    ///
    /// Every change advances the revision. Observations created under an older
    /// revision are discarded. A reduction in authorization also asks the sink
    /// to discard queued records before the new policy becomes visible.
    pub fn apply_policy(&self, policy: PolicySnapshot) {
        let mut current = self
            .telemetry
            .inner
            .policy
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let should_discard = is_policy_reduction(*current, policy);
        self.telemetry
            .inner
            .policy_revision
            .fetch_add(1, Ordering::AcqRel);
        if should_discard {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.telemetry.inner.sink.discard_pending();
            }));
        }
        *current = policy;
    }

    /// Stops new observations while preserving already queued records for a
    /// bounded graceful flush. Consent revocation must use [`Self::apply_policy`]
    /// so the sink discards queued records instead.
    pub fn close_ingress_for_shutdown(&self) {
        let mut current = self
            .telemetry
            .inner
            .policy
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.telemetry
            .inner
            .policy_revision
            .fetch_add(1, Ordering::AcqRel);
        *current = PolicySnapshot::default();
    }

    pub fn policy_snapshot(&self) -> PolicySnapshot {
        self.telemetry.policy_snapshot()
    }

    pub fn diagnostics(&self) -> TelemetryDiagnostics {
        self.telemetry.diagnostics()
    }
}

impl Default for Telemetry {
    fn default() -> Self {
        Self::noop()
    }
}

pub struct TelemetrySpan {
    telemetry: Telemetry,
    schema: &'static OperationSchema,
    policy_revision: u64,
    started_at: Instant,
    started_unix_nanos: u128,
    context: Option<SpanContext>,
    parent_span_id: Option<[u8; 8]>,
    links: Vec<SpanContext>,
    attributes: Vec<Attribute>,
    active: bool,
    closed: bool,
}

impl std::fmt::Debug for TelemetrySpan {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TelemetrySpan")
            .field("name", &self.schema.span_name)
            .field("context", &self.context)
            .field("active", &self.active)
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}

impl TelemetrySpan {
    fn disabled(
        telemetry: Telemetry,
        schema: &'static OperationSchema,
        policy_revision: u64,
    ) -> Self {
        Self {
            telemetry,
            schema,
            policy_revision,
            started_at: Instant::now(),
            started_unix_nanos: 0,
            context: None,
            parent_span_id: None,
            links: Vec::new(),
            attributes: Vec::new(),
            active: false,
            closed: false,
        }
    }

    pub fn context(&self) -> Option<SpanContext> {
        self.context
    }

    pub(crate) fn finish(
        mut self,
        attributes: Vec<Attribute>,
        status: SpanStatus,
        severity: Severity,
    ) {
        let telemetry = self.telemetry.clone();
        telemetry.finish_span(&mut self, attributes, status, severity);
    }
}

impl Drop for TelemetrySpan {
    fn drop(&mut self) {
        if self.active && !self.closed {
            let telemetry = self.telemetry.clone();
            telemetry.finish_span(
                self,
                vec![
                    Attribute::enumeration("bitfun.outcome", "incomplete"),
                    Attribute::enumeration("error.type", "internal"),
                ],
                SpanStatus::Unset,
                Severity::Warn,
            );
        }
    }
}

fn unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn level_rank(level: TelemetryLevel) -> u8 {
    match level {
        TelemetryLevel::Off => 0,
        TelemetryLevel::Basic => 1,
        TelemetryLevel::Diagnostic => 2,
    }
}

fn is_policy_reduction(current: PolicySnapshot, next: PolicySnapshot) -> bool {
    level_rank(next.level) < level_rank(current.level)
        || (trace_authorized(current) && !trace_authorized(next))
        || (metric_authorized(current) && !metric_authorized(next))
        || (log_authorized(current) && !log_authorized(next))
}

fn trace_authorized(policy: PolicySnapshot) -> bool {
    policy.level == TelemetryLevel::Diagnostic && policy.signals.traces
}

fn metric_authorized(policy: PolicySnapshot) -> bool {
    policy.level != TelemetryLevel::Off && policy.signals.metrics
}

fn log_authorized(policy: PolicySnapshot) -> bool {
    policy.level != TelemetryLevel::Off && policy.signals.logs
}

fn should_emit_log(
    policy: PolicySnapshot,
    sampling: SamplingClass,
    severity: Severity,
    sequence: u64,
) -> bool {
    if matches!(severity, Severity::Warn | Severity::Error) || sampling == SamplingClass::Always {
        return true;
    }
    let ratio = policy.success_log_sample_ratio.clamp(0.0, 1.0);
    if ratio <= 0.0 {
        return false;
    }
    if ratio >= 1.0 {
        return true;
    }
    let bucket = sequence % 10_000;
    bucket < (ratio * 10_000.0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domains::{
        start_turn, AgentModeClass, CompletionFacts, Entrypoint, PriorityClass, TurnFinishFacts,
        TurnStartFacts, TurnTrigger,
    };
    use crate::InMemorySink;

    struct PanicSink;

    impl TelemetrySink for PanicSink {
        fn emit(&self, _record: ValidatedRecord) {
            panic!("simulated sink failure");
        }
    }

    #[test]
    fn enabling_from_off_with_a_signal_disabled_is_not_a_policy_reduction() {
        let next = PolicySnapshot::new(TelemetryLevel::Diagnostic)
            .with_signals(SignalPolicy::new(false, true, true));

        assert!(!is_policy_reduction(PolicySnapshot::default(), next));
        assert!(is_policy_reduction(
            PolicySnapshot::new(TelemetryLevel::Diagnostic),
            next
        ));
    }

    #[test]
    fn off_is_a_true_noop() {
        let sink = Arc::new(InMemorySink::default());
        let (telemetry, _control) = Telemetry::build(PolicySnapshot::default(), sink.clone());
        start_turn(
            &telemetry,
            TurnStartFacts {
                entrypoint: Entrypoint::Cli,
                mode_class: AgentModeClass::Agentic,
                trigger: TurnTrigger::User,
                priority_class: PriorityClass::Interactive,
                remote: false,
                subagent: false,
            },
            None,
        )
        .finish(TurnFinishFacts {
            completion: CompletionFacts::completed(),
            finish_reason: None,
            round_count: 1,
            tool_count: 0,
        });
        assert!(sink.is_empty());
    }

    #[test]
    fn policy_change_discards_queued_and_stale_observations() {
        let sink = Arc::new(InMemorySink::default());
        let (telemetry, control) = Telemetry::build(
            PolicySnapshot::new(TelemetryLevel::Diagnostic)
                .with_trace_sample_ratio(1.0)
                .with_success_log_sample_ratio(1.0),
            sink.clone(),
        );
        let observation = start_turn(
            &telemetry,
            TurnStartFacts {
                entrypoint: Entrypoint::Desktop,
                mode_class: AgentModeClass::Agentic,
                trigger: TurnTrigger::User,
                priority_class: PriorityClass::Interactive,
                remote: false,
                subagent: false,
            },
            None,
        );
        control.apply_policy(PolicySnapshot::default());
        observation.finish(TurnFinishFacts {
            completion: CompletionFacts::completed(),
            finish_reason: None,
            round_count: 1,
            tool_count: 0,
        });
        assert!(sink.is_empty());
        assert!(telemetry.diagnostics().skipped() > 0);
    }

    #[test]
    fn graceful_shutdown_closes_ingress_without_discarding_queued_records() {
        let sink = Arc::new(InMemorySink::default());
        let (telemetry, control) = Telemetry::build(
            PolicySnapshot::new(TelemetryLevel::Basic).with_success_log_sample_ratio(1.0),
            sink.clone(),
        );
        start_turn(
            &telemetry,
            TurnStartFacts {
                entrypoint: Entrypoint::Cli,
                mode_class: AgentModeClass::Agentic,
                trigger: TurnTrigger::User,
                priority_class: PriorityClass::Interactive,
                remote: false,
                subagent: false,
            },
            None,
        )
        .finish(TurnFinishFacts {
            completion: CompletionFacts::completed(),
            finish_reason: None,
            round_count: 1,
            tool_count: 0,
        });
        let queued_before_shutdown = sink.records().len();

        control.close_ingress_for_shutdown();

        assert!(queued_before_shutdown > 0);
        assert_eq!(sink.records().len(), queued_before_shutdown);
        assert_eq!(control.policy_snapshot().level(), TelemetryLevel::Off);
    }

    #[test]
    fn sink_failure_does_not_escape_into_business_control_flow() {
        let (telemetry, _control) = Telemetry::build(
            PolicySnapshot::new(TelemetryLevel::Basic).with_success_log_sample_ratio(1.0),
            Arc::new(PanicSink),
        );
        start_turn(
            &telemetry,
            TurnStartFacts {
                entrypoint: Entrypoint::Cli,
                mode_class: AgentModeClass::Agentic,
                trigger: TurnTrigger::User,
                priority_class: PriorityClass::Interactive,
                remote: false,
                subagent: false,
            },
            None,
        )
        .finish(TurnFinishFacts {
            completion: CompletionFacts::completed(),
            finish_reason: None,
            round_count: 1,
            tool_count: 0,
        });
        assert_eq!(telemetry.diagnostics().rejected(), 3);
    }
}
