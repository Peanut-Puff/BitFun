//! Typed business facts for the first observability rollout.
//!
//! These APIs intentionally expose only stable classes, counts, booleans, and
//! durations. They have no slots for prompts, model payloads, tool arguments,
//! paths, user/device identity, endpoints, raw errors, or extension names.

use crate::schema::OperationKind;
use crate::{Attribute, Severity, SpanContext, SpanStatus, Telemetry, TelemetrySpan};

macro_rules! safe_enum {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value),+
                }
            }
        }
    };
}

safe_enum!(Entrypoint {
    Desktop => "desktop",
    Cli => "cli",
    Server => "server",
    Relay => "relay",
    Web => "web",
});
safe_enum!(PlatformClass {
    Macos => "macos",
    Windows => "windows",
    Linux => "linux",
    Ohos => "ohos",
    Other => "other",
});
safe_enum!(Outcome {
    Completed => "completed",
    Failed => "failed",
    Cancelled => "cancelled",
    Timeout => "timeout",
    Rejected => "rejected",
    Blocked => "blocked",
    Degraded => "degraded",
    Incomplete => "incomplete",
});
safe_enum!(SafeErrorType {
    Cancelled => "cancelled",
    Timeout => "timeout",
    Authentication => "authentication",
    RateLimited => "rate_limited",
    NetworkUnavailable => "network_unavailable",
    NetworkProtocol => "network_protocol",
    InvalidRequest => "invalid_request",
    ContextOverflow => "context_overflow",
    ToolValidation => "tool_validation",
    PermissionDenied => "permission_denied",
    Persistence => "persistence",
    Provider => "provider",
    Internal => "internal",
    Other => "other",
});
safe_enum!(StartupPhase {
    Bootstrap => "bootstrap",
    Config => "config",
    Runtime => "runtime",
    Ui => "ui",
    Ready => "ready",
    Shutdown => "shutdown",
});
safe_enum!(RuntimeState {
    Started => "started",
    Ready => "ready",
    Stopped => "stopped",
    Degraded => "degraded",
});
safe_enum!(SessionOperation {
    Create => "create",
    Restore => "restore",
    Close => "close",
});
safe_enum!(SessionKind {
    Interactive => "interactive",
    Background => "background",
    Subagent => "subagent",
    Review => "review",
});
safe_enum!(WorkspaceKind {
    Local => "local",
    Remote => "remote",
    None => "none",
});
safe_enum!(AgentModeClass {
    Agentic => "agentic",
    Chat => "chat",
    Review => "review",
    Goal => "goal",
    Custom => "custom",
});
safe_enum!(TurnTrigger {
    User => "user",
    Continuation => "continuation",
    Scheduled => "scheduled",
    Remote => "remote",
    System => "system",
});
safe_enum!(PriorityClass {
    Interactive => "interactive",
    Normal => "normal",
    Background => "background",
});
safe_enum!(FinishReasonClass {
    Completed => "completed",
    ToolCalls => "tool_calls",
    Cancelled => "cancelled",
    Length => "length",
    ContentFilter => "content_filter",
    Error => "error",
    Other => "other",
});
safe_enum!(IndexBucket {
    One => "1",
    Two => "2",
    ThreeToFive => "3_5",
    SixToTen => "6_10",
    ElevenPlus => "11_plus",
});
safe_enum!(AttemptBucket {
    One => "1",
    Two => "2",
    ThreePlus => "3_plus",
});
safe_enum!(ProviderClass {
    OpenAiCompatible => "openai_compatible",
    AnthropicCompatible => "anthropic_compatible",
    GoogleCompatible => "google_compatible",
    Local => "local",
    Other => "other",
});
safe_enum!(ModelClass {
    GeneralReasoning => "general_reasoning",
    Fast => "fast",
    Code => "code",
    Vision => "vision",
    Embedding => "embedding",
    Other => "other",
});
safe_enum!(InferenceProtocolClass {
    Responses => "responses",
    ChatCompletions => "chat_completions",
    Messages => "messages",
    Gemini => "gemini",
    Other => "other",
});
safe_enum!(StatusClass {
    None => "none",
    Success => "2xx",
    Redirect => "3xx",
    ClientError => "4xx",
    ServerError => "5xx",
    Network => "network",
});
safe_enum!(ToolClass {
    BuiltIn => "built_in",
    Custom => "custom",
});
safe_enum!(ToolKind {
    Filesystem => "filesystem",
    Search => "search",
    Shell => "shell",
    Git => "git",
    Browser => "browser",
    ComputerUse => "computer_use",
    Protocol => "protocol",
    Task => "task",
    Other => "other",
});
safe_enum!(PermissionKind {
    FilesystemRead => "filesystem_read",
    FilesystemWrite => "filesystem_write",
    Shell => "shell",
    Network => "network",
    Browser => "browser",
    ComputerUse => "computer_use",
    Other => "other",
});
safe_enum!(PermissionDecision {
    AllowOnce => "allow_once",
    AllowSession => "allow_session",
    AllowAlways => "allow_always",
    Deny => "deny",
    Cancelled => "cancelled",
});
safe_enum!(ScopeClass {
    Operation => "operation",
    Session => "session",
    Workspace => "workspace",
    Global => "global",
});
safe_enum!(CompressionTrigger {
    Automatic => "automatic",
    Manual => "manual",
    Recovery => "recovery",
});
safe_enum!(SummarySourceClass {
    Model => "model",
    LocalFallback => "local_fallback",
    None => "none",
});
safe_enum!(GoalOperation {
    Create => "create",
    Restore => "restore",
    Complete => "complete",
    Block => "block",
    Cancel => "cancel",
});
safe_enum!(ReviewStage {
    Prepare => "prepare",
    Analyze => "analyze",
    Verify => "verify",
    Report => "report",
    Overall => "overall",
});
safe_enum!(FindingBucket {
    Zero => "0",
    One => "1",
    TwoToFive => "2_5",
    SixToTwenty => "6_20",
    TwentyOnePlus => "21_plus",
});
safe_enum!(ExtensionClass {
    BuiltIn => "built_in",
    Managed => "managed",
    Project => "project",
    User => "user",
    Custom => "custom",
});
safe_enum!(PluginOperation {
    Discover => "discover",
    Load => "load",
    Invoke => "invoke",
    Reload => "reload",
    Unload => "unload",
});
safe_enum!(McpOperation {
    Connect => "connect",
    Initialize => "initialize",
    ListTools => "list_tools",
    CallTool => "call_tool",
    Disconnect => "disconnect",
});
safe_enum!(McpTransport {
    Stdio => "stdio",
    StreamableHttp => "streamable_http",
    Sse => "sse",
    Other => "other",
});
safe_enum!(RemoteOperation {
    Connect => "connect",
    Reconnect => "reconnect",
    Disconnect => "disconnect",
    Invoke => "invoke",
    Sync => "sync",
    Heartbeat => "heartbeat",
});
safe_enum!(RemoteTransport {
    Relay => "relay",
    Lan => "lan",
    Ssh => "ssh",
    Peer => "peer",
    Other => "other",
});
safe_enum!(TokenDirection {
    Input => "input",
    Output => "output",
    Reasoning => "reasoning",
    CacheRead => "cache_read",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompletionFacts {
    outcome: Outcome,
    error_type: Option<SafeErrorType>,
}

impl CompletionFacts {
    pub const fn completed() -> Self {
        Self {
            outcome: Outcome::Completed,
            error_type: None,
        }
    }

    pub const fn failed(error_type: SafeErrorType) -> Self {
        Self {
            outcome: Outcome::Failed,
            error_type: Some(error_type),
        }
    }

    pub const fn cancelled() -> Self {
        Self {
            outcome: Outcome::Cancelled,
            error_type: Some(SafeErrorType::Cancelled),
        }
    }

    pub const fn timed_out() -> Self {
        Self {
            outcome: Outcome::Timeout,
            error_type: Some(SafeErrorType::Timeout),
        }
    }

    pub const fn rejected(error_type: SafeErrorType) -> Self {
        Self {
            outcome: Outcome::Rejected,
            error_type: Some(error_type),
        }
    }

    pub const fn blocked(error_type: SafeErrorType) -> Self {
        Self {
            outcome: Outcome::Blocked,
            error_type: Some(error_type),
        }
    }

    pub const fn degraded(error_type: SafeErrorType) -> Self {
        Self {
            outcome: Outcome::Degraded,
            error_type: Some(error_type),
        }
    }

    pub fn outcome(&self) -> Outcome {
        self.outcome
    }

    pub fn error_type(&self) -> Option<SafeErrorType> {
        self.error_type
    }
}

fn completion_parts(
    completion: CompletionFacts,
    mut attributes: Vec<Attribute>,
) -> (Vec<Attribute>, SpanStatus, Severity) {
    attributes.push(Attribute::enumeration(
        "bitfun.outcome",
        completion.outcome.as_str(),
    ));
    if let Some(error_type) = completion.error_type {
        attributes.push(Attribute::enumeration("error.type", error_type.as_str()));
    }
    let (status, severity) = match completion.outcome {
        Outcome::Completed => (SpanStatus::Ok, Severity::Info),
        Outcome::Cancelled => (SpanStatus::Unset, Severity::Info),
        Outcome::Rejected | Outcome::Blocked | Outcome::Degraded | Outcome::Incomplete => {
            (SpanStatus::Unset, Severity::Warn)
        }
        Outcome::Failed | Outcome::Timeout => (SpanStatus::Error, Severity::Error),
    };
    (attributes, status, severity)
}

trait FinishFacts {
    fn into_parts(self) -> (Vec<Attribute>, SpanStatus, Severity);
}

macro_rules! observation {
    ($name:ident, $finish:ty) => {
        #[derive(Debug)]
        pub struct $name(TelemetrySpan);

        impl $name {
            pub fn context(&self) -> Option<SpanContext> {
                self.0.context()
            }

            pub fn finish(self, facts: $finish) {
                let (attributes, status, severity) = facts.into_parts();
                self.0.finish(attributes, status, severity);
            }
        }
    };
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupStartFacts {
    pub app_version: String,
    pub platform: PlatformClass,
    pub entrypoint: Entrypoint,
    pub phase: StartupPhase,
    pub state: RuntimeState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartupFinishFacts {
    pub completion: CompletionFacts,
}

impl FinishFacts for StartupFinishFacts {
    fn into_parts(self) -> (Vec<Attribute>, SpanStatus, Severity) {
        completion_parts(self.completion, Vec::new())
    }
}

observation!(StartupObservation, StartupFinishFacts);

pub fn start_startup(
    telemetry: &Telemetry,
    facts: StartupStartFacts,
    parent: Option<SpanContext>,
) -> StartupObservation {
    StartupObservation(telemetry.start_operation(
        OperationKind::Startup,
        vec![
            Attribute::version("service.version", facts.app_version),
            Attribute::enumeration("bitfun.platform.class", facts.platform.as_str()),
            Attribute::enumeration("bitfun.entrypoint", facts.entrypoint.as_str()),
            Attribute::enumeration("bitfun.phase", facts.phase.as_str()),
            Attribute::enumeration("bitfun.state", facts.state.as_str()),
        ],
        parent,
        Vec::new(),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionStartFacts {
    pub operation: SessionOperation,
    pub kind: SessionKind,
    pub workspace_kind: WorkspaceKind,
    pub mode_class: AgentModeClass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionFinishFacts {
    pub completion: CompletionFacts,
}

impl FinishFacts for SessionFinishFacts {
    fn into_parts(self) -> (Vec<Attribute>, SpanStatus, Severity) {
        completion_parts(self.completion, Vec::new())
    }
}

observation!(SessionObservation, SessionFinishFacts);

pub fn start_session(
    telemetry: &Telemetry,
    facts: SessionStartFacts,
    parent: Option<SpanContext>,
) -> SessionObservation {
    SessionObservation(telemetry.start_operation(
        OperationKind::Session,
        vec![
            Attribute::enumeration("bitfun.session.operation", facts.operation.as_str()),
            Attribute::enumeration("bitfun.session.kind", facts.kind.as_str()),
            Attribute::enumeration("bitfun.workspace.kind", facts.workspace_kind.as_str()),
            Attribute::enumeration("bitfun.agent.mode_class", facts.mode_class.as_str()),
        ],
        parent,
        Vec::new(),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnStartFacts {
    pub entrypoint: Entrypoint,
    pub mode_class: AgentModeClass,
    pub trigger: TurnTrigger,
    pub priority_class: PriorityClass,
    pub remote: bool,
    pub subagent: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnFinishFacts {
    pub completion: CompletionFacts,
    pub finish_reason: Option<FinishReasonClass>,
    pub round_count: u64,
    pub tool_count: u64,
}

impl FinishFacts for TurnFinishFacts {
    fn into_parts(self) -> (Vec<Attribute>, SpanStatus, Severity) {
        let mut attributes = vec![
            Attribute::u64("bitfun.turn.round_count", self.round_count),
            Attribute::u64("bitfun.turn.tool_count", self.tool_count),
        ];
        if let Some(reason) = self.finish_reason {
            attributes.push(Attribute::enumeration(
                "bitfun.finish_reason.class",
                reason.as_str(),
            ));
        }
        completion_parts(self.completion, attributes)
    }
}

observation!(TurnObservation, TurnFinishFacts);

pub fn start_turn(
    telemetry: &Telemetry,
    facts: TurnStartFacts,
    parent: Option<SpanContext>,
) -> TurnObservation {
    TurnObservation(telemetry.start_operation(
        OperationKind::Turn,
        vec![
            Attribute::enumeration("bitfun.entrypoint", facts.entrypoint.as_str()),
            Attribute::enumeration("bitfun.agent.mode_class", facts.mode_class.as_str()),
            Attribute::enumeration("bitfun.turn.trigger", facts.trigger.as_str()),
            Attribute::enumeration("bitfun.priority.class", facts.priority_class.as_str()),
            Attribute::boolean("bitfun.turn.remote", facts.remote),
            Attribute::boolean("bitfun.turn.subagent", facts.subagent),
        ],
        parent,
        Vec::new(),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoundStartFacts {
    pub index_bucket: IndexBucket,
    pub subagent: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoundFinishFacts {
    pub completion: CompletionFacts,
    pub has_tool_calls: bool,
    pub attempt_bucket: AttemptBucket,
}

impl FinishFacts for RoundFinishFacts {
    fn into_parts(self) -> (Vec<Attribute>, SpanStatus, Severity) {
        completion_parts(
            self.completion,
            vec![
                Attribute::boolean("bitfun.round.has_tool_calls", self.has_tool_calls),
                Attribute::enumeration("bitfun.attempt.bucket", self.attempt_bucket.as_str()),
            ],
        )
    }
}

observation!(RoundObservation, RoundFinishFacts);

pub fn start_round(
    telemetry: &Telemetry,
    facts: RoundStartFacts,
    parent: Option<SpanContext>,
) -> RoundObservation {
    RoundObservation(telemetry.start_operation(
        OperationKind::Round,
        vec![
            Attribute::enumeration("bitfun.round.index_bucket", facts.index_bucket.as_str()),
            Attribute::boolean("bitfun.turn.subagent", facts.subagent),
        ],
        parent,
        Vec::new(),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InferenceStartFacts {
    pub provider_class: ProviderClass,
    pub model_class: ModelClass,
    pub protocol_class: InferenceProtocolClass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InferenceFinishFacts {
    pub completion: CompletionFacts,
    pub attempt_bucket: AttemptBucket,
    pub status_class: StatusClass,
    pub retryable: bool,
    pub ttft_ms: Option<u64>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_read_tokens: u64,
}

impl FinishFacts for InferenceFinishFacts {
    fn into_parts(self) -> (Vec<Attribute>, SpanStatus, Severity) {
        let mut attributes = vec![
            Attribute::enumeration("bitfun.attempt.bucket", self.attempt_bucket.as_str()),
            Attribute::enumeration("http.response.status_class", self.status_class.as_str()),
            Attribute::boolean("bitfun.retryable", self.retryable),
            Attribute::u64("bitfun.token.input", self.input_tokens),
            Attribute::u64("bitfun.token.output", self.output_tokens),
            Attribute::u64("bitfun.token.reasoning", self.reasoning_tokens),
            Attribute::u64("bitfun.token.cache_read", self.cache_read_tokens),
        ];
        if let Some(ttft_ms) = self.ttft_ms {
            attributes.push(Attribute::u64("bitfun.inference.ttft_ms", ttft_ms));
        }
        completion_parts(self.completion, attributes)
    }
}

observation!(InferenceObservation, InferenceFinishFacts);

pub fn start_inference(
    telemetry: &Telemetry,
    facts: InferenceStartFacts,
    parent: Option<SpanContext>,
) -> InferenceObservation {
    InferenceObservation(telemetry.start_operation(
        OperationKind::Inference,
        vec![
            Attribute::enumeration("bitfun.provider.class", facts.provider_class.as_str()),
            Attribute::enumeration("bitfun.model.class", facts.model_class.as_str()),
            Attribute::enumeration("bitfun.protocol.class", facts.protocol_class.as_str()),
        ],
        parent,
        Vec::new(),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenUsageFacts {
    pub direction: TokenDirection,
    pub provider_class: ProviderClass,
    pub model_class: ModelClass,
    pub subagent: bool,
    pub tokens: u64,
}

pub fn record_token_usage(telemetry: &Telemetry, facts: TokenUsageFacts) {
    telemetry.record_token_metric(
        facts.tokens,
        vec![
            Attribute::enumeration("bitfun.token.direction", facts.direction.as_str()),
            Attribute::enumeration("bitfun.provider.class", facts.provider_class.as_str()),
            Attribute::enumeration("bitfun.model.class", facts.model_class.as_str()),
            Attribute::boolean("bitfun.turn.subagent", facts.subagent),
        ],
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolStartFacts {
    pub tool_class: ToolClass,
    pub tool_kind: ToolKind,
    pub parallel: bool,
    pub remote: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolFinishFacts {
    pub completion: CompletionFacts,
    pub queue_ms: u64,
    pub preflight_ms: u64,
}

impl FinishFacts for ToolFinishFacts {
    fn into_parts(self) -> (Vec<Attribute>, SpanStatus, Severity) {
        completion_parts(
            self.completion,
            vec![
                Attribute::u64("bitfun.tool.queue_ms", self.queue_ms),
                Attribute::u64("bitfun.tool.preflight_ms", self.preflight_ms),
            ],
        )
    }
}

observation!(ToolObservation, ToolFinishFacts);

pub fn start_tool(
    telemetry: &Telemetry,
    facts: ToolStartFacts,
    parent: Option<SpanContext>,
) -> ToolObservation {
    ToolObservation(telemetry.start_operation(
        OperationKind::Tool,
        vec![
            Attribute::enumeration("bitfun.tool.class", facts.tool_class.as_str()),
            Attribute::enumeration("bitfun.tool.kind", facts.tool_kind.as_str()),
            Attribute::boolean("bitfun.tool.parallel", facts.parallel),
            Attribute::boolean("bitfun.tool.remote", facts.remote),
        ],
        parent,
        Vec::new(),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionStartFacts {
    pub kind: PermissionKind,
    pub interactive: bool,
    pub scope_class: ScopeClass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionFinishFacts {
    pub completion: CompletionFacts,
    pub decision: PermissionDecision,
}

impl FinishFacts for PermissionFinishFacts {
    fn into_parts(self) -> (Vec<Attribute>, SpanStatus, Severity) {
        completion_parts(
            self.completion,
            vec![Attribute::enumeration(
                "bitfun.permission.decision",
                self.decision.as_str(),
            )],
        )
    }
}

observation!(PermissionObservation, PermissionFinishFacts);

pub fn start_permission(
    telemetry: &Telemetry,
    facts: PermissionStartFacts,
    parent: Option<SpanContext>,
) -> PermissionObservation {
    PermissionObservation(telemetry.start_operation(
        OperationKind::Permission,
        vec![
            Attribute::enumeration("bitfun.permission.kind", facts.kind.as_str()),
            Attribute::boolean("bitfun.permission.interactive", facts.interactive),
            Attribute::enumeration("bitfun.permission.scope_class", facts.scope_class.as_str()),
        ],
        parent,
        Vec::new(),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompressionStartFacts {
    pub trigger: CompressionTrigger,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompressionFinishFacts {
    pub completion: CompletionFacts,
    pub summary_source: SummarySourceClass,
    pub tokens_before: u64,
    pub tokens_after: u64,
    pub compression_ratio: f64,
}

impl FinishFacts for CompressionFinishFacts {
    fn into_parts(self) -> (Vec<Attribute>, SpanStatus, Severity) {
        completion_parts(
            self.completion,
            vec![
                Attribute::enumeration(
                    "bitfun.context.summary_source",
                    self.summary_source.as_str(),
                ),
                Attribute::u64("bitfun.token.before", self.tokens_before),
                Attribute::u64("bitfun.token.after", self.tokens_after),
                Attribute::f64("bitfun.context.compression_ratio", self.compression_ratio),
            ],
        )
    }
}

observation!(CompressionObservation, CompressionFinishFacts);

pub fn start_compression(
    telemetry: &Telemetry,
    facts: CompressionStartFacts,
    parent: Option<SpanContext>,
) -> CompressionObservation {
    CompressionObservation(telemetry.start_operation(
        OperationKind::Compression,
        vec![Attribute::enumeration(
            "bitfun.context.trigger",
            facts.trigger.as_str(),
        )],
        parent,
        Vec::new(),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GoalStartFacts {
    pub operation: GoalOperation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GoalFinishFacts {
    pub completion: CompletionFacts,
}

impl FinishFacts for GoalFinishFacts {
    fn into_parts(self) -> (Vec<Attribute>, SpanStatus, Severity) {
        completion_parts(self.completion, Vec::new())
    }
}

observation!(GoalObservation, GoalFinishFacts);

pub fn start_goal(
    telemetry: &Telemetry,
    facts: GoalStartFacts,
    links: &[SpanContext],
) -> GoalObservation {
    GoalObservation(telemetry.start_operation(
        OperationKind::Goal,
        vec![Attribute::enumeration(
            "bitfun.goal.operation",
            facts.operation.as_str(),
        )],
        None,
        links.to_vec(),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReviewStartFacts {
    pub stage: ReviewStage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReviewFinishFacts {
    pub completion: CompletionFacts,
    pub finding_bucket: FindingBucket,
}

impl FinishFacts for ReviewFinishFacts {
    fn into_parts(self) -> (Vec<Attribute>, SpanStatus, Severity) {
        completion_parts(
            self.completion,
            vec![Attribute::enumeration(
                "bitfun.review.finding_bucket",
                self.finding_bucket.as_str(),
            )],
        )
    }
}

observation!(ReviewObservation, ReviewFinishFacts);

pub fn start_review(
    telemetry: &Telemetry,
    facts: ReviewStartFacts,
    parent: Option<SpanContext>,
) -> ReviewObservation {
    ReviewObservation(telemetry.start_operation(
        OperationKind::Review,
        vec![Attribute::enumeration(
            "bitfun.review.stage",
            facts.stage.as_str(),
        )],
        parent,
        Vec::new(),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HookStartFacts {
    pub extension_class: ExtensionClass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HookFinishFacts {
    pub completion: CompletionFacts,
}

impl FinishFacts for HookFinishFacts {
    fn into_parts(self) -> (Vec<Attribute>, SpanStatus, Severity) {
        completion_parts(self.completion, Vec::new())
    }
}

observation!(HookObservation, HookFinishFacts);

pub fn start_hook(
    telemetry: &Telemetry,
    facts: HookStartFacts,
    parent: Option<SpanContext>,
) -> HookObservation {
    HookObservation(telemetry.start_operation(
        OperationKind::Hook,
        vec![
            Attribute::enumeration("bitfun.extension.operation", "invoke"),
            Attribute::enumeration("bitfun.extension.class", facts.extension_class.as_str()),
        ],
        parent,
        Vec::new(),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PluginStartFacts {
    pub operation: PluginOperation,
    pub extension_class: ExtensionClass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PluginFinishFacts {
    pub completion: CompletionFacts,
}

impl FinishFacts for PluginFinishFacts {
    fn into_parts(self) -> (Vec<Attribute>, SpanStatus, Severity) {
        completion_parts(self.completion, Vec::new())
    }
}

observation!(PluginObservation, PluginFinishFacts);

pub fn start_plugin(
    telemetry: &Telemetry,
    facts: PluginStartFacts,
    parent: Option<SpanContext>,
) -> PluginObservation {
    PluginObservation(telemetry.start_operation(
        OperationKind::Plugin,
        vec![
            Attribute::enumeration("bitfun.extension.operation", facts.operation.as_str()),
            Attribute::enumeration("bitfun.extension.class", facts.extension_class.as_str()),
        ],
        parent,
        Vec::new(),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpStartFacts {
    pub operation: McpOperation,
    pub transport: McpTransport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpFinishFacts {
    pub completion: CompletionFacts,
    pub attempt_bucket: AttemptBucket,
}

impl FinishFacts for McpFinishFacts {
    fn into_parts(self) -> (Vec<Attribute>, SpanStatus, Severity) {
        completion_parts(
            self.completion,
            vec![Attribute::enumeration(
                "bitfun.attempt.bucket",
                self.attempt_bucket.as_str(),
            )],
        )
    }
}

observation!(McpObservation, McpFinishFacts);

pub fn start_mcp(
    telemetry: &Telemetry,
    facts: McpStartFacts,
    parent: Option<SpanContext>,
) -> McpObservation {
    McpObservation(telemetry.start_operation(
        OperationKind::Mcp,
        vec![
            Attribute::enumeration("bitfun.protocol.operation", facts.operation.as_str()),
            Attribute::enumeration("bitfun.protocol.transport", facts.transport.as_str()),
        ],
        parent,
        Vec::new(),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteStartFacts {
    pub operation: RemoteOperation,
    pub transport: RemoteTransport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteFinishFacts {
    pub completion: CompletionFacts,
    pub attempt_bucket: AttemptBucket,
    pub rtt_ms: Option<u64>,
}

impl FinishFacts for RemoteFinishFacts {
    fn into_parts(self) -> (Vec<Attribute>, SpanStatus, Severity) {
        let mut attributes = vec![Attribute::enumeration(
            "bitfun.attempt.bucket",
            self.attempt_bucket.as_str(),
        )];
        if let Some(rtt_ms) = self.rtt_ms {
            attributes.push(Attribute::u64("bitfun.remote.rtt_ms", rtt_ms));
        }
        completion_parts(self.completion, attributes)
    }
}

observation!(RemoteObservation, RemoteFinishFacts);

pub fn start_remote(
    telemetry: &Telemetry,
    facts: RemoteStartFacts,
    parent: Option<SpanContext>,
) -> RemoteObservation {
    RemoteObservation(telemetry.start_operation(
        OperationKind::Remote,
        vec![
            Attribute::enumeration("bitfun.remote.operation", facts.operation.as_str()),
            Attribute::enumeration("bitfun.remote.transport", facts.transport.as_str()),
        ],
        parent,
        Vec::new(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InMemorySink, PolicySnapshot, SignalKind, TelemetryLevel, ValidatedRecord};
    use std::sync::Arc;

    fn diagnostic_telemetry() -> (Telemetry, Arc<InMemorySink>) {
        let sink = Arc::new(InMemorySink::default());
        let (telemetry, _control) = Telemetry::build(
            PolicySnapshot::new(TelemetryLevel::Diagnostic)
                .with_trace_sample_ratio(1.0)
                .with_success_log_sample_ratio(1.0),
            sink.clone(),
        );
        (telemetry, sink)
    }

    #[test]
    fn one_typed_fact_produces_all_three_signals() {
        let (telemetry, sink) = diagnostic_telemetry();
        start_turn(
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
        )
        .finish(TurnFinishFacts {
            completion: CompletionFacts::completed(),
            finish_reason: Some(FinishReasonClass::Completed),
            round_count: 2,
            tool_count: 1,
        });

        let records = sink.records();
        assert_eq!(
            records
                .iter()
                .filter(|record| record.signal_kind() == SignalKind::Trace)
                .count(),
            1
        );
        assert_eq!(
            records
                .iter()
                .filter(|record| record.signal_kind() == SignalKind::Metric)
                .count(),
            2
        );
        assert_eq!(
            records
                .iter()
                .filter(|record| record.signal_kind() == SignalKind::Log)
                .count(),
            1
        );
        assert!(records.iter().all(|record| {
            record.attributes().iter().all(|attribute| {
                !attribute.key().contains("prompt")
                    && !attribute.key().contains("path")
                    && !attribute.key().contains("user")
            })
        }));
    }

    #[test]
    fn child_span_inherits_trace_and_sets_parent() {
        let (telemetry, sink) = diagnostic_telemetry();
        let turn = start_turn(
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
        );
        let parent = turn.context().expect("diagnostic trace context");
        start_round(
            &telemetry,
            RoundStartFacts {
                index_bucket: IndexBucket::One,
                subagent: false,
            },
            Some(parent),
        )
        .finish(RoundFinishFacts {
            completion: CompletionFacts::completed(),
            has_tool_calls: false,
            attempt_bucket: AttemptBucket::One,
        });
        turn.finish(TurnFinishFacts {
            completion: CompletionFacts::completed(),
            finish_reason: Some(FinishReasonClass::Completed),
            round_count: 1,
            tool_count: 0,
        });

        let spans: Vec<_> = sink
            .records()
            .into_iter()
            .filter_map(|record| match record {
                ValidatedRecord::Span(span) => Some(span),
                _ => None,
            })
            .collect();
        let round = spans
            .iter()
            .find(|span| span.name() == "bitfun.agent.round")
            .expect("round span");
        assert_eq!(round.context().trace_id(), parent.trace_id());
        assert_eq!(round.parent_span_id(), Some(parent.span_id()));
    }

    #[test]
    fn unsafe_dynamic_value_never_reaches_the_sink() {
        let (telemetry, sink) = diagnostic_telemetry();
        let canary = "prompt=/Users/alice/private api_key=secret";
        start_startup(
            &telemetry,
            StartupStartFacts {
                app_version: canary.to_string(),
                platform: PlatformClass::Macos,
                entrypoint: Entrypoint::Desktop,
                phase: StartupPhase::Ready,
                state: RuntimeState::Ready,
            },
            None,
        )
        .finish(StartupFinishFacts {
            completion: CompletionFacts::completed(),
        });

        let encoded = serde_json::to_string(&sink.records()).expect("serialize safe records");
        assert!(!encoded.contains(canary));
        assert!(!encoded.contains("/Users/alice"));
        assert!(telemetry.diagnostics().rejected() >= 1);
    }

    #[test]
    fn dropped_observation_uses_a_safe_incomplete_terminal_state() {
        let (telemetry, sink) = diagnostic_telemetry();
        let observation = start_turn(
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
        );
        drop(observation);

        assert!(sink.records().iter().any(|record| {
            record.attributes().iter().any(|attribute| {
                attribute.key() == "bitfun.outcome"
                    && attribute.value() == &crate::AttributeValue::Enum("incomplete".to_string())
            })
        }));
    }

    #[test]
    fn common_feature_descriptors_are_callable_without_freeform_values() {
        let (telemetry, sink) = diagnostic_telemetry();
        start_goal(
            &telemetry,
            GoalStartFacts {
                operation: GoalOperation::Complete,
            },
            &[],
        )
        .finish(GoalFinishFacts {
            completion: CompletionFacts::completed(),
        });
        start_review(
            &telemetry,
            ReviewStartFacts {
                stage: ReviewStage::Overall,
            },
            None,
        )
        .finish(ReviewFinishFacts {
            completion: CompletionFacts::completed(),
            finding_bucket: FindingBucket::Zero,
        });
        start_hook(
            &telemetry,
            HookStartFacts {
                extension_class: ExtensionClass::BuiltIn,
            },
            None,
        )
        .finish(HookFinishFacts {
            completion: CompletionFacts::completed(),
        });
        start_plugin(
            &telemetry,
            PluginStartFacts {
                operation: PluginOperation::Load,
                extension_class: ExtensionClass::Managed,
            },
            None,
        )
        .finish(PluginFinishFacts {
            completion: CompletionFacts::completed(),
        });
        start_mcp(
            &telemetry,
            McpStartFacts {
                operation: McpOperation::Connect,
                transport: McpTransport::Stdio,
            },
            None,
        )
        .finish(McpFinishFacts {
            completion: CompletionFacts::completed(),
            attempt_bucket: AttemptBucket::One,
        });
        start_remote(
            &telemetry,
            RemoteStartFacts {
                operation: RemoteOperation::Connect,
                transport: RemoteTransport::Relay,
            },
            None,
        )
        .finish(RemoteFinishFacts {
            completion: CompletionFacts::completed(),
            attempt_bucket: AttemptBucket::One,
            rtt_ms: Some(10),
        });

        assert!(sink
            .records()
            .iter()
            .any(|record| record.name() == "bitfun.goal.lifecycle"));
        assert!(sink
            .records()
            .iter()
            .any(|record| record.name() == "bitfun.deep_review.run"));
        assert!(sink
            .records()
            .iter()
            .any(|record| record.name() == "bitfun.hook.invoke"));
        assert!(sink
            .records()
            .iter()
            .any(|record| record.name() == "bitfun.plugin.lifecycle"));
        assert!(sink
            .records()
            .iter()
            .any(|record| record.name() == "bitfun.protocol.operation"));
        assert!(sink
            .records()
            .iter()
            .any(|record| record.name() == "bitfun.remote.operation"));
    }
}
