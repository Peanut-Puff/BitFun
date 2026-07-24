use crate::model::{Attribute, AttributeValue, SignalKind, ValidatedRecord};
use std::collections::HashSet;
use std::sync::OnceLock;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    Enum,
    Bool,
    U64,
    I64,
    F64,
    Version,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SamplingClass {
    Always,
    HighFrequencySuccess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimitClass {
    LowVolume,
    HighFrequency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetentionClass {
    Diagnostic,
    Aggregate,
    Operational,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricUnit {
    One,
    Seconds,
    Tokens,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CardinalityBudget {
    max_attribute_combinations: u32,
}

impl CardinalityBudget {
    pub const fn new(max_attribute_combinations: u32) -> Self {
        Self {
            max_attribute_combinations,
        }
    }

    pub fn max_attribute_combinations(&self) -> u32 {
        self.max_attribute_combinations
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldView {
    key: &'static str,
    field_type: FieldType,
    required: bool,
    enum_values: &'static [&'static str],
    max_length: usize,
    label_eligible: bool,
}

impl FieldView {
    pub fn key(&self) -> &'static str {
        self.key
    }

    pub fn field_type(&self) -> FieldType {
        self.field_type
    }

    pub fn is_required(&self) -> bool {
        self.required
    }

    pub fn enum_values(&self) -> &'static [&'static str] {
        self.enum_values
    }

    pub fn max_length(&self) -> usize {
        self.max_length
    }

    pub fn is_label_eligible(&self) -> bool {
        self.label_eligible
    }
}

const fn enum_field(
    key: &'static str,
    required: bool,
    values: &'static [&'static str],
    label_eligible: bool,
) -> FieldView {
    FieldView {
        key,
        field_type: FieldType::Enum,
        required,
        enum_values: values,
        max_length: 32,
        label_eligible,
    }
}

const fn bool_field(key: &'static str, required: bool, label_eligible: bool) -> FieldView {
    FieldView {
        key,
        field_type: FieldType::Bool,
        required,
        enum_values: &[],
        max_length: 0,
        label_eligible,
    }
}

const fn u64_field(key: &'static str, required: bool) -> FieldView {
    FieldView {
        key,
        field_type: FieldType::U64,
        required,
        enum_values: &[],
        max_length: 0,
        label_eligible: false,
    }
}

const fn f64_field(key: &'static str, required: bool) -> FieldView {
    FieldView {
        key,
        field_type: FieldType::F64,
        required,
        enum_values: &[],
        max_length: 0,
        label_eligible: false,
    }
}

const fn version_field(key: &'static str, required: bool) -> FieldView {
    FieldView {
        key,
        field_type: FieldType::Version,
        required,
        enum_values: &[],
        max_length: 64,
        label_eligible: false,
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DescriptorView {
    name: &'static str,
    version: u16,
    signal: SignalKind,
    minimum_level: crate::TelemetryLevel,
    owner: &'static str,
    fields: &'static [FieldView],
    cardinality_budget: CardinalityBudget,
    sampling: SamplingClass,
    rate_limit: RateLimitClass,
    retention: RetentionClass,
    metric_unit: Option<MetricUnit>,
    consumer: &'static str,
    body: Option<&'static str>,
    deprecated_by: Option<&'static str>,
}

impl DescriptorView {
    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn version(&self) -> u16 {
        self.version
    }

    pub fn signal(&self) -> SignalKind {
        self.signal
    }

    pub fn minimum_level(&self) -> crate::TelemetryLevel {
        self.minimum_level
    }

    pub fn owner(&self) -> &'static str {
        self.owner
    }

    pub fn fields(&self) -> &'static [FieldView] {
        self.fields
    }

    pub fn cardinality_budget(&self) -> CardinalityBudget {
        self.cardinality_budget
    }

    pub fn sampling(&self) -> SamplingClass {
        self.sampling
    }

    pub fn rate_limit(&self) -> RateLimitClass {
        self.rate_limit
    }

    pub fn retention(&self) -> RetentionClass {
        self.retention
    }

    pub fn metric_unit(&self) -> Option<MetricUnit> {
        self.metric_unit
    }

    pub fn consumer(&self) -> &'static str {
        self.consumer
    }

    pub fn body(&self) -> Option<&'static str> {
        self.body
    }

    pub fn deprecated_by(&self) -> Option<&'static str> {
        self.deprecated_by
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OperationKind {
    Startup,
    Session,
    Turn,
    Round,
    Inference,
    Tool,
    Permission,
    Compression,
    Goal,
    Review,
    Hook,
    Plugin,
    Mcp,
    Remote,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct OperationSchema {
    pub kind: OperationKind,
    pub span_name: &'static str,
    pub total_metric_name: &'static str,
    pub duration_metric_name: &'static str,
    pub log_name: &'static str,
    pub log_body: &'static str,
    pub owner: &'static str,
    pub consumer: &'static str,
    pub fields: &'static [FieldView],
    pub metric_fields: &'static [FieldView],
    pub cardinality_budget: CardinalityBudget,
    pub sampling: SamplingClass,
}

const ENTRYPOINTS: &[&str] = &["desktop", "cli", "server", "relay", "web"];
const PLATFORMS: &[&str] = &["macos", "windows", "linux", "ohos", "other"];
const OUTCOMES: &[&str] = &[
    "completed",
    "failed",
    "cancelled",
    "timeout",
    "rejected",
    "blocked",
    "degraded",
    "incomplete",
];
const ERROR_TYPES: &[&str] = &[
    "cancelled",
    "timeout",
    "authentication",
    "rate_limited",
    "network_unavailable",
    "network_protocol",
    "invalid_request",
    "context_overflow",
    "tool_validation",
    "permission_denied",
    "persistence",
    "provider",
    "internal",
    "other",
];
const WORKSPACE_KINDS: &[&str] = &["local", "remote", "none"];
const STARTUP_PHASES: &[&str] = &["bootstrap", "config", "runtime", "ui", "ready", "shutdown"];
const RUNTIME_STATES: &[&str] = &["started", "ready", "stopped", "degraded"];
const SESSION_OPERATIONS: &[&str] = &["create", "restore", "close"];
const SESSION_KINDS: &[&str] = &["interactive", "background", "subagent", "review"];
const MODE_CLASSES: &[&str] = &["agentic", "chat", "review", "goal", "custom"];
const TURN_TRIGGERS: &[&str] = &["user", "continuation", "scheduled", "remote", "system"];
const PRIORITY_CLASSES: &[&str] = &["interactive", "normal", "background"];
const FINISH_REASONS: &[&str] = &[
    "completed",
    "tool_calls",
    "cancelled",
    "length",
    "content_filter",
    "error",
    "other",
];
const INDEX_BUCKETS: &[&str] = &["1", "2", "3_5", "6_10", "11_plus"];
const ATTEMPT_BUCKETS: &[&str] = &["1", "2", "3_plus"];
const PROVIDER_CLASSES: &[&str] = &[
    "openai_compatible",
    "anthropic_compatible",
    "google_compatible",
    "local",
    "other",
];
const MODEL_CLASSES: &[&str] = &[
    "general_reasoning",
    "fast",
    "code",
    "vision",
    "embedding",
    "other",
];
const PROTOCOL_CLASSES: &[&str] = &[
    "responses",
    "chat_completions",
    "messages",
    "gemini",
    "other",
];
const STATUS_CLASSES: &[&str] = &["none", "2xx", "3xx", "4xx", "5xx", "network"];
const TOOL_CLASSES: &[&str] = &["built_in", "custom"];
const TOOL_KINDS: &[&str] = &[
    "filesystem",
    "search",
    "shell",
    "git",
    "browser",
    "computer_use",
    "protocol",
    "task",
    "other",
];
const PERMISSION_KINDS: &[&str] = &[
    "filesystem_read",
    "filesystem_write",
    "shell",
    "network",
    "browser",
    "computer_use",
    "other",
];
const PERMISSION_DECISIONS: &[&str] = &[
    "allow_once",
    "allow_session",
    "allow_always",
    "deny",
    "cancelled",
];
const SCOPE_CLASSES: &[&str] = &["operation", "session", "workspace", "global"];
const COMPRESSION_TRIGGERS: &[&str] = &["automatic", "manual", "recovery"];
const SUMMARY_SOURCES: &[&str] = &["model", "local_fallback", "none"];
const GOAL_OPERATIONS: &[&str] = &["create", "restore", "complete", "block", "cancel"];
const REVIEW_STAGES: &[&str] = &["prepare", "analyze", "verify", "report", "overall"];
const FINDING_BUCKETS: &[&str] = &["0", "1", "2_5", "6_20", "21_plus"];
const HOOK_OPERATIONS: &[&str] = &["invoke"];
const PLUGIN_OPERATIONS: &[&str] = &["discover", "load", "invoke", "reload", "unload"];
const EXTENSION_CLASSES: &[&str] = &["built_in", "managed", "project", "user", "custom"];
const MCP_OPERATIONS: &[&str] = &[
    "connect",
    "initialize",
    "list_tools",
    "call_tool",
    "disconnect",
];
const MCP_TRANSPORTS: &[&str] = &["stdio", "streamable_http", "sse", "other"];
const REMOTE_OPERATIONS: &[&str] = &[
    "connect",
    "reconnect",
    "disconnect",
    "invoke",
    "sync",
    "heartbeat",
];
const REMOTE_TRANSPORTS: &[&str] = &["relay", "lan", "ssh", "peer", "other"];

const STARTUP_FIELDS: &[FieldView] = &[
    version_field("service.version", true),
    enum_field("bitfun.platform.class", true, PLATFORMS, false),
    enum_field("bitfun.entrypoint", true, ENTRYPOINTS, true),
    enum_field("bitfun.phase", true, STARTUP_PHASES, true),
    enum_field("bitfun.state", true, RUNTIME_STATES, true),
    enum_field("bitfun.outcome", true, OUTCOMES, true),
    u64_field("bitfun.duration_ms", true),
    enum_field("error.type", false, ERROR_TYPES, true),
];
const STARTUP_METRIC_FIELDS: &[FieldView] = &[
    enum_field("bitfun.entrypoint", true, ENTRYPOINTS, true),
    enum_field("bitfun.phase", true, STARTUP_PHASES, true),
    enum_field("bitfun.outcome", true, OUTCOMES, true),
];
const SESSION_FIELDS: &[FieldView] = &[
    enum_field("bitfun.session.operation", true, SESSION_OPERATIONS, true),
    enum_field("bitfun.session.kind", true, SESSION_KINDS, true),
    enum_field("bitfun.workspace.kind", true, WORKSPACE_KINDS, true),
    enum_field("bitfun.agent.mode_class", true, MODE_CLASSES, true),
    enum_field("bitfun.outcome", true, OUTCOMES, true),
    u64_field("bitfun.duration_ms", true),
    enum_field("error.type", false, ERROR_TYPES, true),
];
const SESSION_METRIC_FIELDS: &[FieldView] = &[
    enum_field("bitfun.session.operation", true, SESSION_OPERATIONS, true),
    enum_field("bitfun.session.kind", true, SESSION_KINDS, true),
    enum_field("bitfun.workspace.kind", true, WORKSPACE_KINDS, true),
    enum_field("bitfun.outcome", true, OUTCOMES, true),
];
const TURN_FIELDS: &[FieldView] = &[
    enum_field("bitfun.entrypoint", true, ENTRYPOINTS, true),
    enum_field("bitfun.agent.mode_class", true, MODE_CLASSES, true),
    enum_field("bitfun.turn.trigger", true, TURN_TRIGGERS, true),
    enum_field("bitfun.priority.class", true, PRIORITY_CLASSES, true),
    bool_field("bitfun.turn.remote", true, true),
    bool_field("bitfun.turn.subagent", true, true),
    enum_field("bitfun.outcome", true, OUTCOMES, true),
    enum_field("bitfun.finish_reason.class", false, FINISH_REASONS, true),
    u64_field("bitfun.turn.round_count", false),
    u64_field("bitfun.turn.tool_count", false),
    u64_field("bitfun.duration_ms", true),
    enum_field("error.type", false, ERROR_TYPES, true),
];
const TURN_METRIC_FIELDS: &[FieldView] = &[
    enum_field("bitfun.entrypoint", true, ENTRYPOINTS, true),
    enum_field("bitfun.agent.mode_class", true, MODE_CLASSES, true),
    bool_field("bitfun.turn.remote", true, true),
    bool_field("bitfun.turn.subagent", true, true),
    enum_field("bitfun.outcome", true, OUTCOMES, true),
];
const ROUND_FIELDS: &[FieldView] = &[
    enum_field("bitfun.round.index_bucket", true, INDEX_BUCKETS, true),
    bool_field("bitfun.turn.subagent", true, true),
    bool_field("bitfun.round.has_tool_calls", false, true),
    enum_field("bitfun.attempt.bucket", false, ATTEMPT_BUCKETS, true),
    enum_field("bitfun.outcome", true, OUTCOMES, true),
    u64_field("bitfun.duration_ms", true),
    enum_field("error.type", false, ERROR_TYPES, true),
];
const ROUND_METRIC_FIELDS: &[FieldView] = &[
    enum_field("bitfun.round.index_bucket", true, INDEX_BUCKETS, true),
    bool_field("bitfun.turn.subagent", true, true),
    enum_field("bitfun.outcome", true, OUTCOMES, true),
];
const INFERENCE_FIELDS: &[FieldView] = &[
    enum_field("bitfun.provider.class", true, PROVIDER_CLASSES, true),
    enum_field("bitfun.model.class", true, MODEL_CLASSES, true),
    enum_field("bitfun.protocol.class", true, PROTOCOL_CLASSES, true),
    enum_field("bitfun.attempt.bucket", false, ATTEMPT_BUCKETS, true),
    enum_field("http.response.status_class", false, STATUS_CLASSES, true),
    bool_field("bitfun.retryable", false, true),
    enum_field("bitfun.outcome", true, OUTCOMES, true),
    u64_field("bitfun.inference.ttft_ms", false),
    u64_field("bitfun.token.input", false),
    u64_field("bitfun.token.output", false),
    u64_field("bitfun.token.reasoning", false),
    u64_field("bitfun.token.cache_read", false),
    u64_field("bitfun.duration_ms", true),
    enum_field("error.type", false, ERROR_TYPES, true),
];
const INFERENCE_METRIC_FIELDS: &[FieldView] = &[
    enum_field("bitfun.provider.class", true, PROVIDER_CLASSES, true),
    enum_field("bitfun.model.class", true, MODEL_CLASSES, true),
    enum_field("bitfun.protocol.class", true, PROTOCOL_CLASSES, true),
    enum_field("bitfun.outcome", true, OUTCOMES, true),
];
const TOOL_FIELDS: &[FieldView] = &[
    enum_field("bitfun.tool.class", true, TOOL_CLASSES, true),
    enum_field("bitfun.tool.kind", true, TOOL_KINDS, true),
    bool_field("bitfun.tool.parallel", true, true),
    bool_field("bitfun.tool.remote", true, true),
    enum_field("bitfun.outcome", true, OUTCOMES, true),
    u64_field("bitfun.tool.queue_ms", false),
    u64_field("bitfun.tool.preflight_ms", false),
    u64_field("bitfun.duration_ms", true),
    enum_field("error.type", false, ERROR_TYPES, true),
];
const TOOL_METRIC_FIELDS: &[FieldView] = &[
    enum_field("bitfun.tool.class", true, TOOL_CLASSES, true),
    enum_field("bitfun.tool.kind", true, TOOL_KINDS, true),
    bool_field("bitfun.tool.remote", true, true),
    enum_field("bitfun.outcome", true, OUTCOMES, true),
];
const PERMISSION_FIELDS: &[FieldView] = &[
    enum_field("bitfun.permission.kind", true, PERMISSION_KINDS, true),
    bool_field("bitfun.permission.interactive", true, true),
    enum_field("bitfun.permission.scope_class", true, SCOPE_CLASSES, true),
    enum_field(
        "bitfun.permission.decision",
        false,
        PERMISSION_DECISIONS,
        true,
    ),
    enum_field("bitfun.outcome", true, OUTCOMES, true),
    u64_field("bitfun.duration_ms", true),
    enum_field("error.type", false, ERROR_TYPES, true),
];
const PERMISSION_METRIC_FIELDS: &[FieldView] = &[
    enum_field("bitfun.permission.kind", true, PERMISSION_KINDS, true),
    bool_field("bitfun.permission.interactive", true, true),
    enum_field(
        "bitfun.permission.decision",
        false,
        PERMISSION_DECISIONS,
        true,
    ),
];
const COMPRESSION_FIELDS: &[FieldView] = &[
    enum_field("bitfun.context.trigger", true, COMPRESSION_TRIGGERS, true),
    enum_field(
        "bitfun.context.summary_source",
        false,
        SUMMARY_SOURCES,
        true,
    ),
    enum_field("bitfun.outcome", true, OUTCOMES, true),
    u64_field("bitfun.token.before", false),
    u64_field("bitfun.token.after", false),
    f64_field("bitfun.context.compression_ratio", false),
    u64_field("bitfun.duration_ms", true),
    enum_field("error.type", false, ERROR_TYPES, true),
];
const COMPRESSION_METRIC_FIELDS: &[FieldView] = &[
    enum_field("bitfun.context.trigger", true, COMPRESSION_TRIGGERS, true),
    enum_field(
        "bitfun.context.summary_source",
        false,
        SUMMARY_SOURCES,
        true,
    ),
    enum_field("bitfun.outcome", true, OUTCOMES, true),
];
const GOAL_FIELDS: &[FieldView] = &[
    enum_field("bitfun.goal.operation", true, GOAL_OPERATIONS, true),
    enum_field("bitfun.outcome", true, OUTCOMES, true),
    u64_field("bitfun.duration_ms", true),
    enum_field("error.type", false, ERROR_TYPES, true),
];
const GOAL_METRIC_FIELDS: &[FieldView] = &[
    enum_field("bitfun.goal.operation", true, GOAL_OPERATIONS, true),
    enum_field("bitfun.outcome", true, OUTCOMES, true),
];
const REVIEW_FIELDS: &[FieldView] = &[
    enum_field("bitfun.review.stage", true, REVIEW_STAGES, true),
    enum_field("bitfun.review.finding_bucket", false, FINDING_BUCKETS, true),
    enum_field("bitfun.outcome", true, OUTCOMES, true),
    u64_field("bitfun.duration_ms", true),
    enum_field("error.type", false, ERROR_TYPES, true),
];
const REVIEW_METRIC_FIELDS: &[FieldView] = &[
    enum_field("bitfun.review.stage", true, REVIEW_STAGES, true),
    enum_field("bitfun.review.finding_bucket", false, FINDING_BUCKETS, true),
    enum_field("bitfun.outcome", true, OUTCOMES, true),
];
const HOOK_FIELDS: &[FieldView] = &[
    enum_field("bitfun.extension.operation", true, HOOK_OPERATIONS, true),
    enum_field("bitfun.extension.class", true, EXTENSION_CLASSES, true),
    enum_field("bitfun.outcome", true, OUTCOMES, true),
    u64_field("bitfun.duration_ms", true),
    enum_field("error.type", false, ERROR_TYPES, true),
];
const HOOK_METRIC_FIELDS: &[FieldView] = &[
    enum_field("bitfun.extension.class", true, EXTENSION_CLASSES, true),
    enum_field("bitfun.outcome", true, OUTCOMES, true),
];
const PLUGIN_FIELDS: &[FieldView] = &[
    enum_field("bitfun.extension.operation", true, PLUGIN_OPERATIONS, true),
    enum_field("bitfun.extension.class", true, EXTENSION_CLASSES, true),
    enum_field("bitfun.outcome", true, OUTCOMES, true),
    u64_field("bitfun.duration_ms", true),
    enum_field("error.type", false, ERROR_TYPES, true),
];
const PLUGIN_METRIC_FIELDS: &[FieldView] = &[
    enum_field("bitfun.extension.operation", true, PLUGIN_OPERATIONS, true),
    enum_field("bitfun.extension.class", true, EXTENSION_CLASSES, true),
    enum_field("bitfun.outcome", true, OUTCOMES, true),
];
const MCP_FIELDS: &[FieldView] = &[
    enum_field("bitfun.protocol.operation", true, MCP_OPERATIONS, true),
    enum_field("bitfun.protocol.transport", true, MCP_TRANSPORTS, true),
    enum_field("bitfun.attempt.bucket", false, ATTEMPT_BUCKETS, true),
    enum_field("bitfun.outcome", true, OUTCOMES, true),
    u64_field("bitfun.duration_ms", true),
    enum_field("error.type", false, ERROR_TYPES, true),
];
const MCP_METRIC_FIELDS: &[FieldView] = &[
    enum_field("bitfun.protocol.operation", true, MCP_OPERATIONS, true),
    enum_field("bitfun.protocol.transport", true, MCP_TRANSPORTS, true),
    enum_field("bitfun.outcome", true, OUTCOMES, true),
];
const REMOTE_FIELDS: &[FieldView] = &[
    enum_field("bitfun.remote.operation", true, REMOTE_OPERATIONS, true),
    enum_field("bitfun.remote.transport", true, REMOTE_TRANSPORTS, true),
    enum_field("bitfun.attempt.bucket", false, ATTEMPT_BUCKETS, true),
    enum_field("bitfun.outcome", true, OUTCOMES, true),
    u64_field("bitfun.remote.rtt_ms", false),
    u64_field("bitfun.duration_ms", true),
    enum_field("error.type", false, ERROR_TYPES, true),
];
const REMOTE_METRIC_FIELDS: &[FieldView] = &[
    enum_field("bitfun.remote.operation", true, REMOTE_OPERATIONS, true),
    enum_field("bitfun.remote.transport", true, REMOTE_TRANSPORTS, true),
    enum_field("bitfun.outcome", true, OUTCOMES, true),
];
const TOKEN_METRIC_FIELDS: &[FieldView] = &[
    enum_field(
        "bitfun.token.direction",
        true,
        &["input", "output", "reasoning", "cache_read"],
        true,
    ),
    enum_field("bitfun.provider.class", true, PROVIDER_CLASSES, true),
    enum_field("bitfun.model.class", true, MODEL_CLASSES, true),
    bool_field("bitfun.turn.subagent", true, true),
];

const OPERATION_SCHEMAS: &[OperationSchema] = &[
    operation(
        OperationKind::Startup,
        "bitfun.app.startup",
        "bitfun.app.startup.total",
        "bitfun.app.startup.duration",
        "bitfun.app.lifecycle",
        "Application lifecycle changed",
        "app entrypoint",
        "startup regression",
        STARTUP_FIELDS,
        STARTUP_METRIC_FIELDS,
        256,
        SamplingClass::Always,
    ),
    operation(
        OperationKind::Session,
        "bitfun.session.operation",
        "bitfun.session.operation.total",
        "bitfun.session.operation.duration",
        "bitfun.session.lifecycle",
        "Session lifecycle operation finished",
        "session manager",
        "session reliability",
        SESSION_FIELDS,
        SESSION_METRIC_FIELDS,
        512,
        SamplingClass::Always,
    ),
    operation(
        OperationKind::Turn,
        "bitfun.agent.turn",
        "bitfun.agent.turn.total",
        "bitfun.agent.turn.duration",
        "bitfun.agent.turn",
        "Agent turn finished",
        "execution engine",
        "agent quality and latency",
        TURN_FIELDS,
        TURN_METRIC_FIELDS,
        2048,
        SamplingClass::HighFrequencySuccess,
    ),
    operation(
        OperationKind::Round,
        "bitfun.agent.round",
        "bitfun.agent.round.total",
        "bitfun.agent.round.duration",
        "bitfun.agent.round",
        "Agent round finished",
        "round executor",
        "round reliability",
        ROUND_FIELDS,
        ROUND_METRIC_FIELDS,
        256,
        SamplingClass::HighFrequencySuccess,
    ),
    operation(
        OperationKind::Inference,
        "bitfun.inference.request",
        "bitfun.inference.request.total",
        "bitfun.inference.request.duration",
        "bitfun.inference.request",
        "Inference request finished",
        "round executor",
        "provider latency and cost",
        INFERENCE_FIELDS,
        INFERENCE_METRIC_FIELDS,
        2048,
        SamplingClass::HighFrequencySuccess,
    ),
    operation(
        OperationKind::Tool,
        "bitfun.tool.execute",
        "bitfun.tool.execute.total",
        "bitfun.tool.duration",
        "bitfun.tool.execution",
        "Tool execution finished",
        "tool pipeline",
        "tool reliability",
        TOOL_FIELDS,
        TOOL_METRIC_FIELDS,
        2048,
        SamplingClass::HighFrequencySuccess,
    ),
    operation(
        OperationKind::Permission,
        "bitfun.permission.evaluate",
        "bitfun.permission.decision.total",
        "bitfun.permission.wait",
        "bitfun.permission.decision",
        "Permission decision recorded",
        "permission manager",
        "permission friction",
        PERMISSION_FIELDS,
        PERMISSION_METRIC_FIELDS,
        512,
        SamplingClass::Always,
    ),
    operation(
        OperationKind::Compression,
        "bitfun.context.compact",
        "bitfun.context.compaction.total",
        "bitfun.context.compaction.duration",
        "bitfun.context.operation",
        "Context compaction finished",
        "execution engine",
        "context pressure",
        COMPRESSION_FIELDS,
        COMPRESSION_METRIC_FIELDS,
        128,
        SamplingClass::Always,
    ),
    operation(
        OperationKind::Goal,
        "bitfun.goal.lifecycle",
        "bitfun.goal.lifecycle.total",
        "bitfun.goal.lifecycle.duration",
        "bitfun.goal.lifecycle",
        "Goal lifecycle operation finished",
        "thread goal owner",
        "goal reliability",
        GOAL_FIELDS,
        GOAL_METRIC_FIELDS,
        128,
        SamplingClass::Always,
    ),
    operation(
        OperationKind::Review,
        "bitfun.deep_review.run",
        "bitfun.deep_review.stage.total",
        "bitfun.deep_review.stage.duration",
        "bitfun.deep_review.lifecycle",
        "Deep review stage finished",
        "deep review owner",
        "review quality and latency",
        REVIEW_FIELDS,
        REVIEW_METRIC_FIELDS,
        256,
        SamplingClass::Always,
    ),
    operation(
        OperationKind::Hook,
        "bitfun.hook.invoke",
        "bitfun.hook.invoke.total",
        "bitfun.hook.invoke.duration",
        "bitfun.extension.invocation",
        "Extension invocation finished",
        "hook invocation owner",
        "extension reliability",
        HOOK_FIELDS,
        HOOK_METRIC_FIELDS,
        128,
        SamplingClass::HighFrequencySuccess,
    ),
    operation(
        OperationKind::Plugin,
        "bitfun.plugin.lifecycle",
        "bitfun.plugin.lifecycle.total",
        "bitfun.plugin.lifecycle.duration",
        "bitfun.extension.lifecycle",
        "Extension lifecycle operation finished",
        "plugin runtime host",
        "plugin reliability",
        PLUGIN_FIELDS,
        PLUGIN_METRIC_FIELDS,
        512,
        SamplingClass::Always,
    ),
    operation(
        OperationKind::Mcp,
        "bitfun.protocol.operation",
        "bitfun.protocol.operation.total",
        "bitfun.protocol.operation.duration",
        "bitfun.protocol.operation",
        "Protocol operation finished",
        "mcp runtime",
        "protocol reliability",
        MCP_FIELDS,
        MCP_METRIC_FIELDS,
        1024,
        SamplingClass::HighFrequencySuccess,
    ),
    operation(
        OperationKind::Remote,
        "bitfun.remote.operation",
        "bitfun.remote.operation.total",
        "bitfun.remote.operation.duration",
        "bitfun.remote.operation",
        "Remote operation finished",
        "remote runtime",
        "remote reliability",
        REMOTE_FIELDS,
        REMOTE_METRIC_FIELDS,
        1024,
        SamplingClass::Always,
    ),
];

#[allow(clippy::too_many_arguments)]
const fn operation(
    kind: OperationKind,
    span_name: &'static str,
    total_metric_name: &'static str,
    duration_metric_name: &'static str,
    log_name: &'static str,
    log_body: &'static str,
    owner: &'static str,
    consumer: &'static str,
    fields: &'static [FieldView],
    metric_fields: &'static [FieldView],
    budget: u32,
    sampling: SamplingClass,
) -> OperationSchema {
    OperationSchema {
        kind,
        span_name,
        total_metric_name,
        duration_metric_name,
        log_name,
        log_body,
        owner,
        consumer,
        fields,
        metric_fields,
        cardinality_budget: CardinalityBudget::new(budget),
        sampling,
    }
}

pub(crate) fn operation_schema(kind: OperationKind) -> &'static OperationSchema {
    OPERATION_SCHEMAS
        .iter()
        .find(|schema| schema.kind == kind)
        .expect("all operation kinds are registered")
}

pub fn descriptor_registry() -> &'static [DescriptorView] {
    static REGISTRY: OnceLock<Vec<DescriptorView>> = OnceLock::new();
    REGISTRY
        .get_or_init(|| {
            let mut descriptors = Vec::with_capacity(OPERATION_SCHEMAS.len() * 4 + 1);
            for schema in OPERATION_SCHEMAS {
                descriptors.push(descriptor(
                    schema,
                    SignalKind::Trace,
                    schema.span_name,
                    schema.fields,
                    None,
                    None,
                ));
                descriptors.push(descriptor(
                    schema,
                    SignalKind::Metric,
                    schema.total_metric_name,
                    schema.metric_fields,
                    None,
                    Some(MetricUnit::One),
                ));
                descriptors.push(descriptor(
                    schema,
                    SignalKind::Metric,
                    schema.duration_metric_name,
                    schema.metric_fields,
                    None,
                    Some(MetricUnit::Seconds),
                ));
                descriptors.push(descriptor(
                    schema,
                    SignalKind::Log,
                    schema.log_name,
                    schema.fields,
                    Some(schema.log_body),
                    None,
                ));
            }
            descriptors.push(DescriptorView {
                name: "bitfun.inference.tokens",
                version: 1,
                signal: SignalKind::Metric,
                minimum_level: crate::TelemetryLevel::Basic,
                owner: "round executor",
                fields: TOKEN_METRIC_FIELDS,
                cardinality_budget: CardinalityBudget::new(512),
                sampling: SamplingClass::Always,
                rate_limit: RateLimitClass::HighFrequency,
                retention: RetentionClass::Aggregate,
                metric_unit: Some(MetricUnit::Tokens),
                consumer: "provider usage and cost",
                body: None,
                deprecated_by: None,
            });
            descriptors
        })
        .as_slice()
}

fn descriptor(
    schema: &'static OperationSchema,
    signal: SignalKind,
    name: &'static str,
    fields: &'static [FieldView],
    body: Option<&'static str>,
    metric_unit: Option<MetricUnit>,
) -> DescriptorView {
    DescriptorView {
        name,
        version: 1,
        signal,
        minimum_level: if signal == SignalKind::Trace {
            crate::TelemetryLevel::Diagnostic
        } else {
            crate::TelemetryLevel::Basic
        },
        owner: schema.owner,
        fields,
        cardinality_budget: schema.cardinality_budget,
        sampling: schema.sampling,
        rate_limit: if schema.sampling == SamplingClass::HighFrequencySuccess {
            RateLimitClass::HighFrequency
        } else {
            RateLimitClass::LowVolume
        },
        retention: match signal {
            SignalKind::Trace => RetentionClass::Diagnostic,
            SignalKind::Metric => RetentionClass::Aggregate,
            SignalKind::Log => RetentionClass::Operational,
        },
        metric_unit,
        consumer: schema.consumer,
        body,
        deprecated_by: None,
    }
}

pub(crate) fn token_descriptor() -> &'static DescriptorView {
    descriptor_registry()
        .iter()
        .find(|descriptor| {
            descriptor.signal == SignalKind::Metric && descriptor.name == "bitfun.inference.tokens"
        })
        .expect("token descriptor is registered")
}

pub(crate) fn descriptor_for_record(record: &ValidatedRecord) -> Option<&'static DescriptorView> {
    descriptor_registry().iter().find(|descriptor| {
        descriptor.signal == record.signal_kind() && descriptor.name == record.name()
    })
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum PrivacyError {
    #[error("unknown telemetry descriptor: {signal:?}/{name}")]
    UnknownDescriptor {
        signal: SignalKind,
        name: &'static str,
    },
    #[error("descriptor version mismatch for {name}: expected {expected}, got {actual}")]
    VersionMismatch {
        name: &'static str,
        expected: u16,
        actual: u16,
    },
    #[error("duplicate telemetry field {field} in {name}")]
    DuplicateField {
        name: &'static str,
        field: &'static str,
    },
    #[error("field {field} is not registered for {name}")]
    UnknownField {
        name: &'static str,
        field: &'static str,
    },
    #[error("required telemetry field {field} is missing from {name}")]
    MissingField {
        name: &'static str,
        field: &'static str,
    },
    #[error("telemetry field {field} has the wrong type")]
    TypeMismatch { field: &'static str },
    #[error("telemetry field {field} contains an unregistered enum value")]
    InvalidEnum { field: &'static str },
    #[error("telemetry field {field} exceeds its maximum length")]
    TextTooLong { field: &'static str },
    #[error("telemetry field {field} contains an unsafe version value")]
    InvalidVersion { field: &'static str },
    #[error("telemetry field {field} contains a non-finite number")]
    NonFinite { field: &'static str },
}

pub(crate) fn validate(record: &ValidatedRecord) -> Result<(), PrivacyError> {
    let Some(descriptor) = descriptor_for_record(record) else {
        return Err(PrivacyError::UnknownDescriptor {
            signal: record.signal_kind(),
            name: record.name(),
        });
    };
    let actual_version = match record {
        ValidatedRecord::Span(record) => record.descriptor_version,
        ValidatedRecord::Metric(record) => record.descriptor_version,
        ValidatedRecord::Log(record) => record.descriptor_version,
    };
    if actual_version != descriptor.version {
        return Err(PrivacyError::VersionMismatch {
            name: descriptor.name,
            expected: descriptor.version,
            actual: actual_version,
        });
    }

    let mut present = HashSet::with_capacity(record.attributes().len());
    for attribute in record.attributes() {
        if !present.insert(attribute.key()) {
            return Err(PrivacyError::DuplicateField {
                name: descriptor.name,
                field: attribute.key(),
            });
        }
        let Some(field) = descriptor
            .fields
            .iter()
            .find(|field| field.key == attribute.key())
        else {
            return Err(PrivacyError::UnknownField {
                name: descriptor.name,
                field: attribute.key(),
            });
        };
        validate_value(field, attribute)?;
    }
    for field in descriptor.fields.iter().filter(|field| field.required) {
        if !present.contains(field.key) {
            return Err(PrivacyError::MissingField {
                name: descriptor.name,
                field: field.key,
            });
        }
    }
    Ok(())
}

fn validate_value(field: &FieldView, attribute: &Attribute) -> Result<(), PrivacyError> {
    match (&field.field_type, attribute.value()) {
        (FieldType::Enum, AttributeValue::Enum(value)) => {
            if value.len() > field.max_length {
                return Err(PrivacyError::TextTooLong { field: field.key });
            }
            if !field.enum_values.iter().any(|allowed| *allowed == value) {
                return Err(PrivacyError::InvalidEnum { field: field.key });
            }
        }
        (FieldType::Bool, AttributeValue::Bool(_))
        | (FieldType::U64, AttributeValue::U64(_))
        | (FieldType::I64, AttributeValue::I64(_)) => {}
        (FieldType::F64, AttributeValue::F64(value)) => {
            if !value.is_finite() {
                return Err(PrivacyError::NonFinite { field: field.key });
            }
        }
        (FieldType::Version, AttributeValue::Version(value)) => {
            if value.is_empty() || value.len() > field.max_length {
                return Err(PrivacyError::TextTooLong { field: field.key });
            }
            if !value
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || ".+-_".contains(character))
            {
                return Err(PrivacyError::InvalidVersion { field: field.key });
            }
        }
        _ => return Err(PrivacyError::TypeMismatch { field: field.key }),
    }
    Ok(())
}

pub(crate) fn filter_attributes(
    attributes: &[Attribute],
    fields: &'static [FieldView],
) -> Vec<Attribute> {
    attributes
        .iter()
        .filter(|attribute| fields.iter().any(|field| field.key == attribute.key()))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{MetricRecord, MetricValue};

    #[test]
    fn registry_keys_are_unique_and_remote_safe() {
        let mut keys = HashSet::new();
        let forbidden_fragments = [
            "prompt",
            "content",
            "payload",
            "path",
            "command",
            "endpoint",
            "url",
            "user.name",
            "host.name",
            "machine",
            "email",
            "account",
            "credential",
            "stack",
            "request.body",
            "response.body",
        ];
        for descriptor in descriptor_registry() {
            assert!(descriptor.name().starts_with("bitfun."));
            assert!(keys.insert((descriptor.signal(), descriptor.name())));
            assert!(!descriptor.consumer().is_empty());
            assert_eq!(
                descriptor.metric_unit().is_some(),
                descriptor.signal() == SignalKind::Metric
            );
            assert_eq!(
                descriptor.body().is_some(),
                descriptor.signal() == SignalKind::Log
            );
            assert!(descriptor.deprecated_by().is_none());
            for field in descriptor.fields() {
                assert!(
                    !forbidden_fragments
                        .iter()
                        .any(|fragment| field.key().contains(fragment)),
                    "unsafe field registered: {}",
                    field.key()
                );
                if field.is_label_eligible() {
                    assert!(matches!(
                        field.field_type(),
                        FieldType::Enum | FieldType::Bool
                    ));
                }
            }
        }
    }

    #[test]
    fn registry_contains_the_complete_day_one_scope() {
        let expected = [
            (SignalKind::Trace, "bitfun.app.startup"),
            (SignalKind::Trace, "bitfun.session.operation"),
            (SignalKind::Trace, "bitfun.agent.turn"),
            (SignalKind::Trace, "bitfun.agent.round"),
            (SignalKind::Trace, "bitfun.inference.request"),
            (SignalKind::Metric, "bitfun.inference.tokens"),
            (SignalKind::Trace, "bitfun.tool.execute"),
            (SignalKind::Trace, "bitfun.permission.evaluate"),
            (SignalKind::Trace, "bitfun.context.compact"),
            (SignalKind::Trace, "bitfun.goal.lifecycle"),
            (SignalKind::Trace, "bitfun.deep_review.run"),
            (SignalKind::Trace, "bitfun.hook.invoke"),
            (SignalKind::Trace, "bitfun.plugin.lifecycle"),
            (SignalKind::Trace, "bitfun.protocol.operation"),
            (SignalKind::Trace, "bitfun.remote.operation"),
        ];
        for (signal, name) in expected {
            assert!(
                descriptor_registry()
                    .iter()
                    .any(|descriptor| descriptor.signal() == signal && descriptor.name() == name),
                "missing descriptor {signal:?}/{name}"
            );
        }
    }

    #[test]
    fn enum_cardinality_stays_within_each_descriptor_budget() {
        for descriptor in descriptor_registry()
            .iter()
            .filter(|descriptor| descriptor.signal() == SignalKind::Metric)
        {
            let combinations = descriptor
                .fields()
                .iter()
                .filter(|field| field.is_label_eligible())
                .map(|field| match field.field_type() {
                    FieldType::Enum => field.enum_values().len() as u64,
                    FieldType::Bool => 2,
                    _ => 1,
                })
                .product::<u64>();
            assert!(
                combinations
                    <= u64::from(descriptor.cardinality_budget().max_attribute_combinations()),
                "{} cardinality {} exceeds budget {}",
                descriptor.name(),
                combinations,
                descriptor.cardinality_budget().max_attribute_combinations()
            );
        }
    }

    #[test]
    fn privacy_gate_rejects_unknown_fields() {
        let record = ValidatedRecord::Metric(MetricRecord {
            descriptor_version: 1,
            name: "bitfun.agent.turn.total",
            timestamp_unix_nanos: 1,
            value: MetricValue::Counter(1),
            attributes: vec![Attribute::version("user.name", "sensitive")],
        });
        assert!(matches!(
            validate(&record),
            Err(PrivacyError::UnknownField {
                field: "user.name",
                ..
            })
        ));
    }

    #[test]
    fn privacy_gate_rejects_dynamic_enum_values() {
        let record = ValidatedRecord::Metric(MetricRecord {
            descriptor_version: 1,
            name: "bitfun.agent.turn.total",
            timestamp_unix_nanos: 1,
            value: MetricValue::Counter(1),
            attributes: vec![
                Attribute::enumeration("bitfun.entrypoint", "desktop"),
                Attribute::enumeration("bitfun.agent.mode_class", "private-agent-name"),
                Attribute::boolean("bitfun.turn.remote", false),
                Attribute::boolean("bitfun.turn.subagent", false),
                Attribute::enumeration("bitfun.outcome", "completed"),
            ],
        });
        assert!(matches!(
            validate(&record),
            Err(PrivacyError::InvalidEnum {
                field: "bitfun.agent.mode_class"
            })
        ));
    }
}
