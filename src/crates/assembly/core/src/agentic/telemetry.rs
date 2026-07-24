//! Privacy-safe projection of authoritative Agent events into telemetry facts.

use crate::agentic::events::{AgenticEvent, DeepReviewQueueStatus, EventSubscriber, ToolEventData};
use bitfun_agent_runtime::event_bus::EventSubscriberResult;
use bitfun_core_types::errors::ErrorCategory;
use bitfun_observability::domains::{
    record_token_usage, start_compression, start_goal, start_inference, start_permission,
    start_review, start_round, start_session, start_tool, start_turn, AgentModeClass,
    AttemptBucket, CompletionFacts, CompressionFinishFacts, CompressionObservation,
    CompressionStartFacts, CompressionTrigger, Entrypoint, FindingBucket, FinishReasonClass,
    GoalFinishFacts, GoalObservation, GoalOperation, GoalStartFacts, IndexBucket,
    InferenceFinishFacts, InferenceObservation, InferenceProtocolClass, InferenceStartFacts,
    ModelClass, PermissionDecision, PermissionFinishFacts, PermissionKind, PermissionObservation,
    PermissionStartFacts, PriorityClass, ProviderClass, ReviewFinishFacts, ReviewObservation,
    ReviewStage, ReviewStartFacts, RoundFinishFacts, RoundObservation, RoundStartFacts,
    SafeErrorType, ScopeClass, SessionFinishFacts, SessionKind, SessionOperation,
    SessionStartFacts, StatusClass, SummarySourceClass, TokenDirection, TokenUsageFacts, ToolClass,
    ToolFinishFacts, ToolKind, ToolObservation, ToolStartFacts, TurnFinishFacts, TurnObservation,
    TurnStartFacts, TurnTrigger, WorkspaceKind,
};
use bitfun_observability::{SpanContext, Telemetry};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy)]
struct SessionFacts {
    kind: SessionKind,
    workspace_kind: WorkspaceKind,
    mode_class: AgentModeClass,
    remote: bool,
    subagent: bool,
    review: bool,
}

impl Default for SessionFacts {
    fn default() -> Self {
        Self {
            kind: SessionKind::Interactive,
            workspace_kind: WorkspaceKind::None,
            mode_class: AgentModeClass::Agentic,
            remote: false,
            subagent: false,
            review: false,
        }
    }
}

struct TurnState {
    session_id: String,
    observation: TurnObservation,
    context: Option<SpanContext>,
    subagent: bool,
}

#[derive(Default)]
struct RoundTokens {
    input: u64,
    output: u64,
    reasoning: u64,
    cache_read: u64,
}

struct RoundState {
    turn_id: String,
    observation: RoundObservation,
    inference: Option<InferenceObservation>,
    finish_facts: Option<RoundFinishFacts>,
    attempts: u32,
    tokens: RoundTokens,
}

struct ToolState {
    turn_id: String,
    round_id: String,
    permission_kind: PermissionKind,
    observation: ToolObservation,
    permission: Option<PermissionObservation>,
}

struct CompressionState {
    turn_id: String,
    observation: CompressionObservation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GoalStatus {
    Active,
    Paused,
    Blocked,
    UsageLimited,
    BudgetLimited,
    Complete,
}

struct ReviewState {
    overall: ReviewObservation,
    active_stage: Option<(ReviewStage, ReviewObservation)>,
    finding_bucket: FindingBucket,
}

#[derive(Default)]
struct ProjectionState {
    sessions: HashMap<String, SessionFacts>,
    turns: HashMap<String, TurnState>,
    rounds: HashMap<String, RoundState>,
    current_round_by_turn: HashMap<String, String>,
    tools: HashMap<String, ToolState>,
    compressions: HashMap<String, CompressionState>,
    goals: HashMap<String, GoalStatus>,
    reviews: HashMap<String, ReviewState>,
    latest_turn_context: HashMap<String, SpanContext>,
}

/// Internal subscriber that derives telemetry from the existing Agent event stream.
///
/// Event identifiers are retained only in memory to correlate lifecycle pairs.
/// They are never added to telemetry records.
pub struct AgentTelemetrySubscriber {
    telemetry: Telemetry,
    entrypoint: Entrypoint,
    state: Mutex<ProjectionState>,
}

impl AgentTelemetrySubscriber {
    pub fn new(telemetry: Telemetry, entrypoint: Entrypoint) -> Self {
        Self {
            telemetry,
            entrypoint,
            state: Mutex::new(ProjectionState::default()),
        }
    }

    fn project(&self, event: &AgenticEvent) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match event {
            AgenticEvent::SessionCreated {
                session_id,
                agent_type,
                workspace_path,
                remote_connection_id,
                remote_ssh_host,
                ..
            } => {
                let remote = remote_connection_id.is_some() || remote_ssh_host.is_some();
                let facts = session_facts(agent_type, workspace_path.is_some(), remote);
                state.sessions.insert(session_id.clone(), facts);
                start_session(
                    &self.telemetry,
                    SessionStartFacts {
                        operation: SessionOperation::Create,
                        kind: facts.kind,
                        workspace_kind: facts.workspace_kind,
                        mode_class: facts.mode_class,
                    },
                    None,
                )
                .finish(SessionFinishFacts {
                    completion: CompletionFacts::completed(),
                });
            }
            AgenticEvent::SessionDeleted { session_id } => {
                let facts = state.sessions.remove(session_id).unwrap_or_default();
                start_session(
                    &self.telemetry,
                    SessionStartFacts {
                        operation: SessionOperation::Close,
                        kind: facts.kind,
                        workspace_kind: facts.workspace_kind,
                        mode_class: facts.mode_class,
                    },
                    None,
                )
                .finish(SessionFinishFacts {
                    completion: CompletionFacts::completed(),
                });
                state.latest_turn_context.remove(session_id);
                state.goals.remove(session_id);
            }
            AgenticEvent::SubagentSessionLinked { session_id, .. } => {
                let session = state.sessions.entry(session_id.clone()).or_default();
                session.kind = SessionKind::Subagent;
                session.subagent = true;
            }
            AgenticEvent::DialogTurnStarted {
                session_id,
                turn_id,
                user_message_metadata,
                ..
            } => {
                let session = if let Some(session) = state.sessions.get(session_id).copied() {
                    session
                } else {
                    let session = SessionFacts::default();
                    state.sessions.insert(session_id.clone(), session);
                    start_session(
                        &self.telemetry,
                        SessionStartFacts {
                            operation: SessionOperation::Restore,
                            kind: session.kind,
                            workspace_kind: session.workspace_kind,
                            mode_class: session.mode_class,
                        },
                        None,
                    )
                    .finish(SessionFinishFacts {
                        completion: CompletionFacts::completed(),
                    });
                    session
                };
                let trigger = turn_trigger(user_message_metadata.as_ref(), session.subagent);
                let observation = start_turn(
                    &self.telemetry,
                    TurnStartFacts {
                        entrypoint: self.entrypoint,
                        mode_class: session.mode_class,
                        trigger,
                        priority_class: priority_class(trigger, session.subagent),
                        remote: session.remote,
                        subagent: session.subagent,
                    },
                    None,
                );
                let context = observation.context();
                state.turns.insert(
                    turn_id.clone(),
                    TurnState {
                        session_id: session_id.clone(),
                        observation,
                        context,
                        subagent: session.subagent,
                    },
                );
                if session.review {
                    let overall = start_review(
                        &self.telemetry,
                        ReviewStartFacts {
                            stage: ReviewStage::Overall,
                        },
                        context,
                    );
                    let prepare = start_review(
                        &self.telemetry,
                        ReviewStartFacts {
                            stage: ReviewStage::Prepare,
                        },
                        overall.context(),
                    );
                    state.reviews.insert(
                        turn_id.clone(),
                        ReviewState {
                            overall,
                            active_stage: Some((ReviewStage::Prepare, prepare)),
                            finding_bucket: FindingBucket::Zero,
                        },
                    );
                }
            }
            AgenticEvent::DialogTurnCompleted {
                turn_id,
                total_rounds,
                total_tools,
                success,
                finish_reason,
                partial_recovery_reason,
                ..
            } => {
                let completion = if success.unwrap_or(true) {
                    if partial_recovery_reason.is_some() {
                        CompletionFacts::degraded(SafeErrorType::Provider)
                    } else {
                        CompletionFacts::completed()
                    }
                } else {
                    CompletionFacts::failed(SafeErrorType::Internal)
                };
                self.finish_turn(
                    &mut state,
                    turn_id,
                    completion,
                    finish_reason.as_deref().map(finish_reason_class),
                    *total_rounds as u64,
                    *total_tools as u64,
                );
            }
            AgenticEvent::DialogTurnCancelled { turn_id, .. } => self.finish_turn(
                &mut state,
                turn_id,
                CompletionFacts::cancelled(),
                Some(FinishReasonClass::Cancelled),
                0,
                0,
            ),
            AgenticEvent::DialogTurnFailed {
                turn_id,
                error_category,
                error_detail,
                ..
            } => {
                let error_type = error_detail
                    .as_ref()
                    .map(|detail| safe_error_category(&detail.category))
                    .or_else(|| error_category.as_ref().map(safe_error_category))
                    .unwrap_or(SafeErrorType::Other);
                self.finish_turn(
                    &mut state,
                    turn_id,
                    CompletionFacts::failed(error_type),
                    Some(FinishReasonClass::Error),
                    0,
                    0,
                );
            }
            AgenticEvent::ModelRoundStarted {
                turn_id,
                round_id,
                round_index,
                effective_model_name,
                ..
            } => {
                if let Some(previous_round_id) = state.current_round_by_turn.get(turn_id).cloned() {
                    if previous_round_id != *round_id {
                        self.finish_round(&mut state, &previous_round_id, None);
                    }
                }
                self.transition_review(&mut state, turn_id, ReviewStage::Analyze);
                let (turn_context, subagent) = state
                    .turns
                    .get(turn_id)
                    .map(|turn| (turn.context, turn.subagent))
                    .unwrap_or((None, false));
                let round = start_round(
                    &self.telemetry,
                    RoundStartFacts {
                        index_bucket: index_bucket(*round_index),
                        subagent,
                    },
                    turn_context,
                );
                let provider_class = provider_class(None, effective_model_name);
                let model_class = model_class(effective_model_name);
                let inference = start_inference(
                    &self.telemetry,
                    InferenceStartFacts {
                        provider_class,
                        model_class,
                        protocol_class: InferenceProtocolClass::Other,
                    },
                    round.context(),
                );
                state
                    .current_round_by_turn
                    .insert(turn_id.clone(), round_id.clone());
                state.rounds.insert(
                    round_id.clone(),
                    RoundState {
                        turn_id: turn_id.clone(),
                        observation: round,
                        inference: Some(inference),
                        finish_facts: None,
                        attempts: 1,
                        tokens: RoundTokens::default(),
                    },
                );
            }
            AgenticEvent::ModelRoundAttemptSuperseded { round_id, .. } => {
                if let Some(round) = state.rounds.get_mut(round_id) {
                    round.attempts = round.attempts.saturating_add(1);
                }
            }
            AgenticEvent::TokenUsageUpdated {
                turn_id,
                effective_model_name,
                input_tokens,
                output_tokens,
                is_subagent,
                cached_tokens,
                token_details,
                ..
            } => {
                let model_class = model_class(effective_model_name);
                let provider_class = provider_class(None, effective_model_name);
                let input = *input_tokens as u64;
                let output = output_tokens.unwrap_or(0) as u64;
                let reasoning = token_detail(token_details.as_ref(), "reasoningTokenCount");
                let cache_read = cached_tokens.unwrap_or(0) as u64;
                record_tokens(
                    &self.telemetry,
                    provider_class,
                    model_class,
                    *is_subagent,
                    input,
                    output,
                    reasoning,
                    cache_read,
                );
                if let Some(round_id) = state.current_round_by_turn.get(turn_id).cloned() {
                    if let Some(round) = state.rounds.get_mut(&round_id) {
                        round.tokens.input = round.tokens.input.saturating_add(input);
                        round.tokens.output = round.tokens.output.saturating_add(output);
                        round.tokens.reasoning = round.tokens.reasoning.saturating_add(reasoning);
                        round.tokens.cache_read =
                            round.tokens.cache_read.saturating_add(cache_read);
                    }
                }
            }
            AgenticEvent::ModelRoundCompleted {
                round_id,
                has_tool_calls,
                first_chunk_ms,
                first_visible_output_ms,
                attempt_count,
                failure_category,
                ..
            } => {
                let mut close_round = false;
                if let Some(round) = state.rounds.get_mut(round_id) {
                    let attempts = attempt_count.unwrap_or(round.attempts).max(round.attempts);
                    let error_type = failure_category
                        .as_deref()
                        .map(safe_error_from_text)
                        .unwrap_or(SafeErrorType::Other);
                    let completion = if failure_category.is_some() {
                        CompletionFacts::failed(error_type)
                    } else {
                        CompletionFacts::completed()
                    };
                    if let Some(inference) = round.inference.take() {
                        inference.finish(InferenceFinishFacts {
                            completion,
                            attempt_bucket: attempt_bucket(attempts),
                            status_class: status_class(failure_category.as_deref()),
                            retryable: failure_category.as_deref().is_some_and(is_retryable_error),
                            ttft_ms: first_visible_output_ms.or(*first_chunk_ms),
                            input_tokens: round.tokens.input,
                            output_tokens: round.tokens.output,
                            reasoning_tokens: round.tokens.reasoning,
                            cache_read_tokens: round.tokens.cache_read,
                        });
                    }
                    round.finish_facts = Some(RoundFinishFacts {
                        completion,
                        has_tool_calls: *has_tool_calls,
                        attempt_bucket: attempt_bucket(attempts),
                    });
                    close_round = !has_tool_calls;
                }
                if close_round {
                    self.finish_round(&mut state, round_id, None);
                }
            }
            AgenticEvent::ToolEvent {
                session_id,
                turn_id,
                round_id,
                tool_event,
                ..
            } => self.project_tool_event(&mut state, session_id, turn_id, round_id, tool_event),
            AgenticEvent::ContextCompressionStarted {
                turn_id,
                compression_id,
                trigger,
                ..
            } => {
                let parent = state.turns.get(turn_id).and_then(|turn| turn.context);
                let observation = start_compression(
                    &self.telemetry,
                    CompressionStartFacts {
                        trigger: compression_trigger(trigger),
                    },
                    parent,
                );
                state.compressions.insert(
                    compression_id.clone(),
                    CompressionState {
                        turn_id: turn_id.clone(),
                        observation,
                    },
                );
            }
            AgenticEvent::ContextCompressionCompleted {
                compression_id,
                tokens_before,
                tokens_after,
                compression_ratio,
                has_summary,
                summary_source,
                ..
            } => {
                if let Some(compression) = state.compressions.remove(compression_id) {
                    compression.observation.finish(CompressionFinishFacts {
                        completion: if *has_summary {
                            CompletionFacts::completed()
                        } else {
                            CompletionFacts::degraded(SafeErrorType::Internal)
                        },
                        summary_source: summary_source_class(summary_source, *has_summary),
                        tokens_before: *tokens_before as u64,
                        tokens_after: *tokens_after as u64,
                        compression_ratio: compression_ratio.clamp(0.0, 1.0),
                    });
                }
            }
            AgenticEvent::ContextCompressionFailed {
                compression_id,
                error,
                ..
            } => {
                if let Some(compression) = state.compressions.remove(compression_id) {
                    compression.observation.finish(CompressionFinishFacts {
                        completion: CompletionFacts::failed(safe_error_from_text(error)),
                        summary_source: SummarySourceClass::None,
                        tokens_before: 0,
                        tokens_after: 0,
                        compression_ratio: 0.0,
                    });
                }
            }
            AgenticEvent::ThreadGoalUpdated { session_id, goal } => {
                self.project_goal(&mut state, session_id, goal.as_ref());
            }
            AgenticEvent::DeepReviewQueueStateChanged {
                turn_id,
                queue_state,
                ..
            } => {
                if matches!(
                    queue_state.status,
                    DeepReviewQueueStatus::QueuedForCapacity | DeepReviewQueueStatus::Running
                ) {
                    self.transition_review(&mut state, turn_id, ReviewStage::Verify);
                }
            }
            _ => {}
        }
    }

    fn project_tool_event(
        &self,
        state: &mut ProjectionState,
        session_id: &str,
        turn_id: &str,
        round_id: &str,
        event: &ToolEventData,
    ) {
        let tool_id = event.tool_id();
        let tool_name = event.effective_tool_name();
        if is_review_report_tool(tool_name) && matches!(event, ToolEventData::Started { .. }) {
            self.transition_review(state, turn_id, ReviewStage::Report);
        }
        if let ToolEventData::Completed { result, .. } = event {
            if is_review_report_tool(tool_name) {
                if let Some(review) = state.reviews.get_mut(turn_id) {
                    review.finding_bucket = finding_bucket(review_finding_count(result));
                }
            }
        }

        if matches!(
            event,
            ToolEventData::Queued { .. }
                | ToolEventData::Started { .. }
                | ToolEventData::Completed { .. }
                | ToolEventData::Failed { .. }
                | ToolEventData::Cancelled { .. }
        ) && !state.tools.contains_key(tool_id)
        {
            self.start_tool(state, session_id, turn_id, round_id, tool_id, tool_name);
        }

        match event {
            ToolEventData::ConfirmationNeeded { .. } => {
                if !state.tools.contains_key(tool_id) {
                    self.start_tool(state, session_id, turn_id, round_id, tool_id, tool_name);
                }
                if let Some(tool) = state.tools.get_mut(tool_id) {
                    if tool.permission.is_none() {
                        tool.permission = Some(start_permission(
                            &self.telemetry,
                            PermissionStartFacts {
                                kind: tool.permission_kind,
                                interactive: true,
                                scope_class: ScopeClass::Operation,
                            },
                            tool.observation.context(),
                        ));
                    }
                }
            }
            ToolEventData::Confirmed { .. } => {
                if let Some(permission) = state
                    .tools
                    .get_mut(tool_id)
                    .and_then(|tool| tool.permission.take())
                {
                    permission.finish(PermissionFinishFacts {
                        completion: CompletionFacts::completed(),
                        decision: PermissionDecision::AllowOnce,
                    });
                }
            }
            ToolEventData::Rejected { .. } => {
                if !state.tools.contains_key(tool_id) {
                    self.start_tool(state, session_id, turn_id, round_id, tool_id, tool_name);
                }
                let tool = state.tools.get_mut(tool_id).expect("tool inserted above");
                let permission = tool.permission.take().unwrap_or_else(|| {
                    start_permission(
                        &self.telemetry,
                        PermissionStartFacts {
                            kind: tool.permission_kind,
                            interactive: false,
                            scope_class: ScopeClass::Operation,
                        },
                        tool.observation.context(),
                    )
                });
                permission.finish(PermissionFinishFacts {
                    completion: CompletionFacts::rejected(SafeErrorType::PermissionDenied),
                    decision: PermissionDecision::Deny,
                });
                self.finish_tool(
                    state,
                    tool_id,
                    CompletionFacts::rejected(SafeErrorType::PermissionDenied),
                    0,
                    0,
                );
            }
            ToolEventData::Completed {
                queue_wait_ms,
                preflight_ms,
                ..
            } => self.finish_tool(
                state,
                tool_id,
                CompletionFacts::completed(),
                queue_wait_ms.unwrap_or(0),
                preflight_ms.unwrap_or(0),
            ),
            ToolEventData::Failed {
                error,
                queue_wait_ms,
                preflight_ms,
                ..
            } => self.finish_tool(
                state,
                tool_id,
                CompletionFacts::failed(safe_error_from_text(error)),
                queue_wait_ms.unwrap_or(0),
                preflight_ms.unwrap_or(0),
            ),
            ToolEventData::Cancelled {
                queue_wait_ms,
                preflight_ms,
                ..
            } => self.finish_tool(
                state,
                tool_id,
                CompletionFacts::cancelled(),
                queue_wait_ms.unwrap_or(0),
                preflight_ms.unwrap_or(0),
            ),
            _ => {}
        }
    }

    fn start_tool(
        &self,
        state: &mut ProjectionState,
        session_id: &str,
        turn_id: &str,
        round_id: &str,
        tool_id: &str,
        tool_name: &str,
    ) {
        let parent = state
            .rounds
            .get(round_id)
            .and_then(|round| round.observation.context())
            .or_else(|| state.turns.get(turn_id).and_then(|turn| turn.context));
        let parallel = state.tools.values().any(|tool| tool.round_id == round_id);
        let remote = state
            .sessions
            .get(session_id)
            .is_some_and(|session| session.remote);
        let kind = tool_kind(tool_name);
        let observation = start_tool(
            &self.telemetry,
            ToolStartFacts {
                tool_class: tool_class(tool_name),
                tool_kind: kind,
                parallel,
                remote,
            },
            parent,
        );
        state.tools.insert(
            tool_id.to_string(),
            ToolState {
                turn_id: turn_id.to_string(),
                round_id: round_id.to_string(),
                permission_kind: permission_kind(tool_name, kind),
                observation,
                permission: None,
            },
        );
    }

    fn finish_tool(
        &self,
        state: &mut ProjectionState,
        tool_id: &str,
        completion: CompletionFacts,
        queue_ms: u64,
        preflight_ms: u64,
    ) {
        if let Some(mut tool) = state.tools.remove(tool_id) {
            if let Some(permission) = tool.permission.take() {
                let (permission_completion, decision) = match completion.outcome() {
                    bitfun_observability::domains::Outcome::Cancelled => {
                        (CompletionFacts::cancelled(), PermissionDecision::Cancelled)
                    }
                    bitfun_observability::domains::Outcome::Completed => {
                        (CompletionFacts::completed(), PermissionDecision::AllowOnce)
                    }
                    _ => (
                        CompletionFacts::failed(
                            completion.error_type().unwrap_or(SafeErrorType::Other),
                        ),
                        PermissionDecision::Deny,
                    ),
                };
                permission.finish(PermissionFinishFacts {
                    completion: permission_completion,
                    decision,
                });
            }
            tool.observation.finish(ToolFinishFacts {
                completion,
                queue_ms,
                preflight_ms,
            });
        }
    }

    fn project_goal(&self, state: &mut ProjectionState, session_id: &str, goal: Option<&Value>) {
        let previous = state.goals.get(session_id).copied();
        let next = goal.and_then(goal_status);
        let operation = match (previous, next) {
            (None, Some(GoalStatus::Active)) => Some(GoalOperation::Create),
            (
                Some(
                    GoalStatus::Paused
                    | GoalStatus::Blocked
                    | GoalStatus::UsageLimited
                    | GoalStatus::BudgetLimited,
                ),
                Some(GoalStatus::Active),
            ) => Some(GoalOperation::Restore),
            (Some(status), Some(GoalStatus::Complete)) if status != GoalStatus::Complete => {
                Some(GoalOperation::Complete)
            }
            (
                Some(status),
                Some(GoalStatus::Blocked | GoalStatus::UsageLimited | GoalStatus::BudgetLimited),
            ) if !matches!(
                status,
                GoalStatus::Blocked | GoalStatus::UsageLimited | GoalStatus::BudgetLimited
            ) =>
            {
                Some(GoalOperation::Block)
            }
            (Some(_), None) => Some(GoalOperation::Cancel),
            _ => None,
        };
        match next {
            Some(status) => {
                state.goals.insert(session_id.to_string(), status);
            }
            None => {
                state.goals.remove(session_id);
            }
        }
        let Some(operation) = operation else {
            return;
        };
        let link = state
            .turns
            .values()
            .find(|turn| turn.session_id == session_id)
            .and_then(|turn| turn.context)
            .or_else(|| state.latest_turn_context.get(session_id).copied());
        let links = link.into_iter().collect::<Vec<_>>();
        let observation: GoalObservation = start_goal(
            &self.telemetry,
            GoalStartFacts { operation },
            links.as_slice(),
        );
        let completion = match operation {
            GoalOperation::Block => CompletionFacts::blocked(SafeErrorType::Other),
            GoalOperation::Cancel => CompletionFacts::cancelled(),
            _ => CompletionFacts::completed(),
        };
        observation.finish(GoalFinishFacts { completion });
    }

    fn transition_review(
        &self,
        state: &mut ProjectionState,
        turn_id: &str,
        next_stage: ReviewStage,
    ) {
        let Some(review) = state.reviews.get_mut(turn_id) else {
            return;
        };
        if review
            .active_stage
            .as_ref()
            .is_some_and(|(stage, _)| *stage == next_stage)
        {
            return;
        }
        if let Some((_, stage)) = review.active_stage.take() {
            stage.finish(ReviewFinishFacts {
                completion: CompletionFacts::completed(),
                finding_bucket: FindingBucket::Zero,
            });
        }
        review.active_stage = Some((
            next_stage,
            start_review(
                &self.telemetry,
                ReviewStartFacts { stage: next_stage },
                review.overall.context(),
            ),
        ));
    }

    fn finish_turn(
        &self,
        state: &mut ProjectionState,
        turn_id: &str,
        completion: CompletionFacts,
        finish_reason: Option<FinishReasonClass>,
        round_count: u64,
        tool_count: u64,
    ) {
        self.finish_turn_children(state, turn_id, completion);
        if let Some(mut review) = state.reviews.remove(turn_id) {
            if let Some((_, stage)) = review.active_stage.take() {
                stage.finish(ReviewFinishFacts {
                    completion,
                    finding_bucket: review.finding_bucket,
                });
            }
            review.overall.finish(ReviewFinishFacts {
                completion,
                finding_bucket: review.finding_bucket,
            });
        }
        if let Some(turn) = state.turns.remove(turn_id) {
            if let Some(context) = turn.context {
                state.latest_turn_context.insert(turn.session_id, context);
            }
            turn.observation.finish(TurnFinishFacts {
                completion,
                finish_reason,
                round_count,
                tool_count,
            });
        }
        state.current_round_by_turn.remove(turn_id);
    }

    fn finish_turn_children(
        &self,
        state: &mut ProjectionState,
        turn_id: &str,
        completion: CompletionFacts,
    ) {
        let tool_ids = state
            .tools
            .iter()
            .filter(|(_, tool)| tool.turn_id == turn_id)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for tool_id in tool_ids {
            self.finish_tool(state, &tool_id, completion, 0, 0);
        }
        let compression_ids = state
            .compressions
            .iter()
            .filter(|(_, compression)| compression.turn_id == turn_id)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for compression_id in compression_ids {
            if let Some(compression) = state.compressions.remove(&compression_id) {
                compression.observation.finish(CompressionFinishFacts {
                    completion,
                    summary_source: SummarySourceClass::None,
                    tokens_before: 0,
                    tokens_after: 0,
                    compression_ratio: 0.0,
                });
            }
        }
        let round_ids = state
            .rounds
            .iter()
            .filter(|(_, round)| round.turn_id == turn_id)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for round_id in round_ids {
            self.finish_round(state, &round_id, Some(completion));
        }
    }

    fn finish_round(
        &self,
        state: &mut ProjectionState,
        round_id: &str,
        completion_override: Option<CompletionFacts>,
    ) {
        let Some(mut round) = state.rounds.remove(round_id) else {
            return;
        };
        let completion = completion_override
            .or(round.finish_facts.map(|facts| facts.completion))
            .unwrap_or_else(|| CompletionFacts::failed(SafeErrorType::Internal));
        if let Some(inference) = round.inference.take() {
            inference.finish(InferenceFinishFacts {
                completion,
                attempt_bucket: attempt_bucket(round.attempts),
                status_class: StatusClass::None,
                retryable: false,
                ttft_ms: None,
                input_tokens: round.tokens.input,
                output_tokens: round.tokens.output,
                reasoning_tokens: round.tokens.reasoning,
                cache_read_tokens: round.tokens.cache_read,
            });
        }
        let mut finish_facts = round.finish_facts.unwrap_or(RoundFinishFacts {
            completion,
            has_tool_calls: false,
            attempt_bucket: attempt_bucket(round.attempts),
        });
        finish_facts.completion = completion;
        round.observation.finish(finish_facts);
        if state
            .current_round_by_turn
            .get(&round.turn_id)
            .is_some_and(|current| current == round_id)
        {
            state.current_round_by_turn.remove(&round.turn_id);
        }
    }
}

#[async_trait::async_trait]
impl EventSubscriber for AgentTelemetrySubscriber {
    async fn on_event(&self, event: &AgenticEvent) -> EventSubscriberResult {
        self.project(event);
        Ok(())
    }
}

fn session_facts(agent_type: &str, has_workspace: bool, remote: bool) -> SessionFacts {
    let agent = agent_type.to_ascii_lowercase();
    let review = agent.contains("review");
    let mode_class = if review {
        AgentModeClass::Review
    } else if agent.contains("goal") {
        AgentModeClass::Goal
    } else if agent.contains("chat") {
        AgentModeClass::Chat
    } else if agent.is_empty() || agent.contains("agentic") {
        AgentModeClass::Agentic
    } else {
        AgentModeClass::Custom
    };
    SessionFacts {
        kind: if review {
            SessionKind::Review
        } else {
            SessionKind::Interactive
        },
        workspace_kind: if remote {
            WorkspaceKind::Remote
        } else if has_workspace {
            WorkspaceKind::Local
        } else {
            WorkspaceKind::None
        },
        mode_class,
        remote,
        subagent: false,
        review,
    }
}

fn metadata_flag(metadata: Option<&Value>, key: &str) -> bool {
    metadata
        .and_then(|value| value.get(key))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn turn_trigger(metadata: Option<&Value>, subagent: bool) -> TurnTrigger {
    if metadata_flag(metadata, "threadGoalContinuation") {
        TurnTrigger::Continuation
    } else if metadata_flag(metadata, "scheduledTask") || metadata_flag(metadata, "cronJob") {
        TurnTrigger::Scheduled
    } else if subagent || metadata_flag(metadata, "maintenanceTurn") {
        TurnTrigger::System
    } else {
        TurnTrigger::User
    }
}

fn priority_class(trigger: TurnTrigger, subagent: bool) -> PriorityClass {
    if subagent || matches!(trigger, TurnTrigger::Scheduled) {
        PriorityClass::Background
    } else if matches!(trigger, TurnTrigger::User) {
        PriorityClass::Interactive
    } else {
        PriorityClass::Normal
    }
}

fn index_bucket(index: usize) -> IndexBucket {
    match index.saturating_add(1) {
        1 => IndexBucket::One,
        2 => IndexBucket::Two,
        3..=5 => IndexBucket::ThreeToFive,
        6..=10 => IndexBucket::SixToTen,
        _ => IndexBucket::ElevenPlus,
    }
}

fn attempt_bucket(attempts: u32) -> AttemptBucket {
    match attempts {
        0 | 1 => AttemptBucket::One,
        2 => AttemptBucket::Two,
        _ => AttemptBucket::ThreePlus,
    }
}

fn model_class(model: &str) -> ModelClass {
    let model = model.to_ascii_lowercase();
    if model.contains("embed") {
        ModelClass::Embedding
    } else if model.contains("vision") || model.contains("vl") {
        ModelClass::Vision
    } else if model.contains("code") || model.contains("coder") {
        ModelClass::Code
    } else if model.contains("mini") || model.contains("flash") || model.contains("haiku") {
        ModelClass::Fast
    } else if model.is_empty() {
        ModelClass::Other
    } else {
        ModelClass::GeneralReasoning
    }
}

fn provider_class(provider: Option<&str>, model: &str) -> ProviderClass {
    let value = format!("{} {}", provider.unwrap_or_default(), model).to_ascii_lowercase();
    if value.contains("anthropic") || value.contains("claude") {
        ProviderClass::AnthropicCompatible
    } else if value.contains("google") || value.contains("gemini") {
        ProviderClass::GoogleCompatible
    } else if value.contains("ollama") || value.contains("local") || value.contains("localhost") {
        ProviderClass::Local
    } else if value.contains("openai") || value.contains("gpt") || value.contains("o1") {
        ProviderClass::OpenAiCompatible
    } else {
        ProviderClass::Other
    }
}

fn token_detail(details: Option<&Value>, key: &str) -> u64 {
    details
        .and_then(|value| value.get(key))
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

#[allow(clippy::too_many_arguments)]
fn record_tokens(
    telemetry: &Telemetry,
    provider_class: ProviderClass,
    model_class: ModelClass,
    subagent: bool,
    input: u64,
    output: u64,
    reasoning: u64,
    cache_read: u64,
) {
    for (direction, tokens) in [
        (TokenDirection::Input, input),
        (TokenDirection::Output, output),
        (TokenDirection::Reasoning, reasoning),
        (TokenDirection::CacheRead, cache_read),
    ] {
        if tokens > 0 {
            record_token_usage(
                telemetry,
                TokenUsageFacts {
                    direction,
                    provider_class,
                    model_class,
                    subagent,
                    tokens,
                },
            );
        }
    }
}

fn tool_class(name: &str) -> ToolClass {
    let name = name.to_ascii_lowercase();
    if name.starts_with("mcp__") || name.starts_with("plugin__") || name.starts_with("custom__") {
        ToolClass::Custom
    } else {
        ToolClass::BuiltIn
    }
}

fn tool_kind(name: &str) -> ToolKind {
    let name = name.to_ascii_lowercase();
    if name.contains("browser") {
        ToolKind::Browser
    } else if name.contains("computer") || name.contains("screenshot") {
        ToolKind::ComputerUse
    } else if name.starts_with("mcp__") || name.contains("protocol") {
        ToolKind::Protocol
    } else if name.contains("git") || name.contains("review") {
        ToolKind::Git
    } else if name.contains("search") || name.contains("grep") || name.contains("glob") {
        ToolKind::Search
    } else if name.contains("shell") || name.contains("command") || name.contains("terminal") {
        ToolKind::Shell
    } else if name.contains("read")
        || name.contains("write")
        || name.contains("edit")
        || name.contains("file")
        || name.contains("directory")
    {
        ToolKind::Filesystem
    } else if name.contains("task") || name.contains("agent") || name.contains("goal") {
        ToolKind::Task
    } else {
        ToolKind::Other
    }
}

fn permission_kind(name: &str, kind: ToolKind) -> PermissionKind {
    let name = name.to_ascii_lowercase();
    match kind {
        ToolKind::Filesystem if name.contains("read") || name.contains("search") => {
            PermissionKind::FilesystemRead
        }
        ToolKind::Filesystem => PermissionKind::FilesystemWrite,
        ToolKind::Shell => PermissionKind::Shell,
        ToolKind::Browser => PermissionKind::Browser,
        ToolKind::ComputerUse => PermissionKind::ComputerUse,
        ToolKind::Protocol => PermissionKind::Network,
        _ => PermissionKind::Other,
    }
}

fn compression_trigger(trigger: &str) -> CompressionTrigger {
    match trigger.to_ascii_lowercase().as_str() {
        "manual" => CompressionTrigger::Manual,
        "recovery" => CompressionTrigger::Recovery,
        _ => CompressionTrigger::Automatic,
    }
}

fn summary_source_class(source: &str, has_summary: bool) -> SummarySourceClass {
    if !has_summary {
        SummarySourceClass::None
    } else if source.to_ascii_lowercase().contains("fallback") {
        SummarySourceClass::LocalFallback
    } else {
        SummarySourceClass::Model
    }
}

fn goal_status(goal: &Value) -> Option<GoalStatus> {
    match goal.get("status")?.as_str()? {
        "active" => Some(GoalStatus::Active),
        "paused" => Some(GoalStatus::Paused),
        "blocked" => Some(GoalStatus::Blocked),
        "usageLimited" | "usage_limited" => Some(GoalStatus::UsageLimited),
        "budgetLimited" | "budget_limited" => Some(GoalStatus::BudgetLimited),
        "complete" => Some(GoalStatus::Complete),
        _ => None,
    }
}

fn review_finding_count(result: &Value) -> usize {
    result
        .get("issues")
        .or_else(|| result.get("findings"))
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0)
}

fn finding_bucket(count: usize) -> FindingBucket {
    match count {
        0 => FindingBucket::Zero,
        1 => FindingBucket::One,
        2..=5 => FindingBucket::TwoToFive,
        6..=20 => FindingBucket::SixToTwenty,
        _ => FindingBucket::TwentyOnePlus,
    }
}

fn is_review_report_tool(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "codereview" | "code_review" | "submitreview" | "submit_review"
    )
}

fn safe_error_category(category: &ErrorCategory) -> SafeErrorType {
    match category {
        ErrorCategory::Network => SafeErrorType::NetworkUnavailable,
        ErrorCategory::Auth => SafeErrorType::Authentication,
        ErrorCategory::RateLimit => SafeErrorType::RateLimited,
        ErrorCategory::ContextOverflow => SafeErrorType::ContextOverflow,
        ErrorCategory::Timeout => SafeErrorType::Timeout,
        ErrorCategory::Permission | ErrorCategory::ContentPolicy => SafeErrorType::PermissionDenied,
        ErrorCategory::InvalidRequest => SafeErrorType::InvalidRequest,
        ErrorCategory::ProviderQuota
        | ErrorCategory::ProviderBilling
        | ErrorCategory::ProviderUnavailable
        | ErrorCategory::ModelError => SafeErrorType::Provider,
        ErrorCategory::Unknown => SafeErrorType::Other,
    }
}

fn safe_error_from_text(error: &str) -> SafeErrorType {
    let error = error.to_ascii_lowercase();
    if error.contains("cancel") || error.contains("abort") {
        SafeErrorType::Cancelled
    } else if error.contains("timeout") || error.contains("timed out") {
        SafeErrorType::Timeout
    } else if error.contains("rate limit") || error.contains("429") {
        SafeErrorType::RateLimited
    } else if error.contains("auth") || error.contains("401") {
        SafeErrorType::Authentication
    } else if error.contains("permission") || error.contains("denied") || error.contains("403") {
        SafeErrorType::PermissionDenied
    } else if error.contains("network") || error.contains("connection") || error.contains("dns") {
        SafeErrorType::NetworkUnavailable
    } else if error.contains("context") && error.contains("overflow") {
        SafeErrorType::ContextOverflow
    } else if error.contains("invalid") || error.contains("validation") {
        SafeErrorType::InvalidRequest
    } else {
        SafeErrorType::Other
    }
}

fn is_retryable_error(error: &str) -> bool {
    matches!(
        safe_error_from_text(error),
        SafeErrorType::Timeout | SafeErrorType::RateLimited | SafeErrorType::NetworkUnavailable
    )
}

fn status_class(failure: Option<&str>) -> StatusClass {
    match failure.map(safe_error_from_text) {
        None => StatusClass::Success,
        Some(SafeErrorType::NetworkUnavailable | SafeErrorType::Timeout) => StatusClass::Network,
        Some(
            SafeErrorType::Authentication
            | SafeErrorType::InvalidRequest
            | SafeErrorType::PermissionDenied
            | SafeErrorType::RateLimited,
        ) => StatusClass::ClientError,
        Some(_) => StatusClass::ServerError,
    }
}

fn finish_reason_class(reason: &str) -> FinishReasonClass {
    match reason {
        "complete" => FinishReasonClass::Completed,
        "tool_calls" => FinishReasonClass::ToolCalls,
        "cancelled" => FinishReasonClass::Cancelled,
        "max_rounds" | "partial_truncated" => FinishReasonClass::Length,
        "content_filter" => FinishReasonClass::ContentFilter,
        "error" | "repeated_tool_failures" => FinishReasonClass::Error,
        _ => FinishReasonClass::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitfun_events::ToolEventIdentity;
    use bitfun_observability::{InMemorySink, PolicySnapshot, SignalKind, TelemetryLevel};
    use std::sync::Arc;

    fn telemetry() -> (Telemetry, Arc<InMemorySink>) {
        let sink = Arc::new(InMemorySink::default());
        let (telemetry, _) = Telemetry::build(
            PolicySnapshot::new(TelemetryLevel::Diagnostic)
                .with_trace_sample_ratio(1.0)
                .with_success_log_sample_ratio(1.0),
            sink.clone(),
        );
        (telemetry, sink)
    }

    fn session_created() -> AgenticEvent {
        AgenticEvent::SessionCreated {
            session_id: "private-session-id".to_string(),
            session_name: "private session name".to_string(),
            agent_type: "agentic".to_string(),
            workspace_path: Some("/Users/alice/private".to_string()),
            remote_connection_id: None,
            remote_ssh_host: None,
        }
    }

    fn turn_started() -> AgenticEvent {
        AgenticEvent::DialogTurnStarted {
            session_id: "private-session-id".to_string(),
            turn_id: "private-turn-id".to_string(),
            turn_index: 1,
            user_input: "secret user prompt".to_string(),
            original_user_input: None,
            user_message_metadata: None,
        }
    }

    #[test]
    fn complete_turn_builds_parented_round_inference_and_tool_records() {
        let (telemetry, sink) = telemetry();
        let subscriber = AgentTelemetrySubscriber::new(telemetry, Entrypoint::Desktop);
        subscriber.project(&session_created());
        subscriber.project(&turn_started());
        subscriber.project(&AgenticEvent::ModelRoundStarted {
            session_id: "private-session-id".to_string(),
            turn_id: "private-turn-id".to_string(),
            round_id: "private-round-id".to_string(),
            round_group_id: None,
            round_index: 1,
            model_config_id: "private-model-config".to_string(),
            effective_model_name: "gpt-5".to_string(),
        });
        subscriber.project(&AgenticEvent::TokenUsageUpdated {
            session_id: "private-session-id".to_string(),
            turn_id: "private-turn-id".to_string(),
            model_config_id: "private-model-config".to_string(),
            effective_model_name: "gpt-5".to_string(),
            input_tokens: 10,
            output_tokens: Some(5),
            total_tokens: 15,
            max_context_tokens: Some(100),
            is_subagent: false,
            cached_tokens: Some(2),
            token_details: Some(serde_json::json!({ "reasoningTokenCount": 3 })),
        });
        subscriber.project(&AgenticEvent::ModelRoundCompleted {
            session_id: "private-session-id".to_string(),
            turn_id: "private-turn-id".to_string(),
            round_id: "private-round-id".to_string(),
            has_tool_calls: true,
            duration_ms: Some(2),
            provider_id: Some("openai".to_string()),
            model_config_id: "private-model-config".to_string(),
            effective_model_name: "gpt-5".to_string(),
            first_chunk_ms: Some(1),
            first_visible_output_ms: Some(1),
            stream_duration_ms: Some(1),
            attempt_count: Some(1),
            failure_category: None,
            token_details: None,
        });
        subscriber.project(&AgenticEvent::ToolEvent {
            session_id: "private-session-id".to_string(),
            turn_id: "private-turn-id".to_string(),
            round_id: "private-round-id".to_string(),
            attempt_id: None,
            attempt_index: None,
            tool_event: ToolEventData::Started {
                identity: ToolEventIdentity::direct("private-tool-id", "Read"),
                params: serde_json::json!({ "path": "/Users/alice/private" }),
                timeout_seconds: None,
            },
        });
        subscriber.project(&AgenticEvent::ToolEvent {
            session_id: "private-session-id".to_string(),
            turn_id: "private-turn-id".to_string(),
            round_id: "private-round-id".to_string(),
            attempt_id: None,
            attempt_index: None,
            tool_event: ToolEventData::Completed {
                identity: ToolEventIdentity::direct("private-tool-id", "Read"),
                result: serde_json::json!({ "content": "secret response" }),
                result_for_assistant: Some("secret tool output".to_string()),
                image_attachments: None,
                duration_ms: 1,
                queue_wait_ms: Some(1),
                preflight_ms: Some(1),
                confirmation_wait_ms: None,
                execution_ms: Some(1),
            },
        });
        subscriber.project(&AgenticEvent::DialogTurnCompleted {
            session_id: "private-session-id".to_string(),
            turn_id: "private-turn-id".to_string(),
            total_rounds: 1,
            total_tools: 1,
            duration_ms: 3,
            partial_recovery_reason: None,
            success: Some(true),
            finish_reason: Some("complete".to_string()),
            has_final_response: Some(true),
        });

        let records = sink.records();
        let span_names = records
            .iter()
            .filter_map(|record| match record {
                bitfun_observability::ValidatedRecord::Span(span) => Some(span.name()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(span_names.contains(&"bitfun.agent.turn"));
        assert!(span_names.contains(&"bitfun.agent.round"));
        assert!(span_names.contains(&"bitfun.inference.request"));
        assert!(span_names.contains(&"bitfun.tool.execute"));
        let span = |name| {
            records.iter().find_map(|record| match record {
                bitfun_observability::ValidatedRecord::Span(span) if span.name() == name => {
                    Some(span)
                }
                _ => None,
            })
        };
        let turn = span("bitfun.agent.turn").expect("turn span");
        let round = span("bitfun.agent.round").expect("round span");
        let inference = span("bitfun.inference.request").expect("inference span");
        let tool = span("bitfun.tool.execute").expect("tool span");
        assert_eq!(round.parent_span_id(), Some(turn.context().span_id()));
        assert_eq!(inference.parent_span_id(), Some(round.context().span_id()));
        assert_eq!(tool.parent_span_id(), Some(round.context().span_id()));
        assert_eq!(
            records
                .iter()
                .filter(|record| record.name() == "bitfun.agent.turn.total")
                .count(),
            1
        );
        let serialized = serde_json::to_string(&records).expect("serialize records");
        for secret in [
            "private-session-id",
            "private-turn-id",
            "private-round-id",
            "private-tool-id",
            "/Users/alice/private",
            "secret user prompt",
            "secret response",
            "secret tool output",
        ] {
            assert!(!serialized.contains(secret), "leaked canary: {secret}");
        }
        assert!(records
            .iter()
            .any(|record| record.signal_kind() == SignalKind::Log));
    }

    #[test]
    fn compression_projects_one_parented_safe_terminal_fact() {
        let (telemetry, sink) = telemetry();
        let subscriber = AgentTelemetrySubscriber::new(telemetry, Entrypoint::Desktop);
        subscriber.project(&session_created());
        subscriber.project(&turn_started());
        subscriber.project(&AgenticEvent::ContextCompressionStarted {
            session_id: "private-session-id".to_string(),
            turn_id: "private-turn-id".to_string(),
            compression_id: "private-compression-id".to_string(),
            trigger: "automatic".to_string(),
            tokens_before: 1_000,
            context_window: 2_000,
        });
        subscriber.project(&AgenticEvent::ContextCompressionCompleted {
            session_id: "private-session-id".to_string(),
            turn_id: "private-turn-id".to_string(),
            compression_id: "private-compression-id".to_string(),
            compression_count: 1,
            tokens_before: 1_000,
            tokens_after: 400,
            compression_ratio: 0.4,
            duration_ms: 10,
            has_summary: true,
            summary_source: "model".to_string(),
        });
        subscriber.project(&AgenticEvent::DialogTurnCompleted {
            session_id: "private-session-id".to_string(),
            turn_id: "private-turn-id".to_string(),
            total_rounds: 0,
            total_tools: 0,
            duration_ms: 10,
            partial_recovery_reason: None,
            success: Some(true),
            finish_reason: Some("complete".to_string()),
            has_final_response: Some(true),
        });

        let records = sink.records();
        let turn = records.iter().find_map(|record| match record {
            bitfun_observability::ValidatedRecord::Span(span)
                if span.name() == "bitfun.agent.turn" =>
            {
                Some(span)
            }
            _ => None,
        });
        let compression = records.iter().find_map(|record| match record {
            bitfun_observability::ValidatedRecord::Span(span)
                if span.name() == "bitfun.context.compact" =>
            {
                Some(span)
            }
            _ => None,
        });
        assert_eq!(
            compression.expect("compression span").parent_span_id(),
            Some(turn.expect("turn span").context().span_id())
        );
        assert_eq!(
            records
                .iter()
                .filter(|record| record.name() == "bitfun.context.compaction.total")
                .count(),
            1
        );
        let serialized = serde_json::to_string(&records).expect("serialize records");
        for secret in [
            "private-session-id",
            "private-turn-id",
            "private-compression-id",
        ] {
            assert!(!serialized.contains(secret), "leaked canary: {secret}");
        }
    }

    #[test]
    fn retry_cancel_reject_goal_block_and_review_are_distinct_terminal_facts() {
        let (telemetry, sink) = telemetry();
        let subscriber = AgentTelemetrySubscriber::new(telemetry, Entrypoint::Cli);
        let mut review_session = session_created();
        if let AgenticEvent::SessionCreated { agent_type, .. } = &mut review_session {
            *agent_type = "DeepReview".to_string();
        }
        subscriber.project(&review_session);
        subscriber.project(&turn_started());
        subscriber.project(&AgenticEvent::ThreadGoalUpdated {
            session_id: "private-session-id".to_string(),
            goal: Some(serde_json::json!({ "status": "active", "objective": "secret" })),
        });
        subscriber.project(&AgenticEvent::ThreadGoalUpdated {
            session_id: "private-session-id".to_string(),
            goal: Some(serde_json::json!({ "status": "blocked", "objective": "secret" })),
        });
        subscriber.project(&AgenticEvent::ModelRoundStarted {
            session_id: "private-session-id".to_string(),
            turn_id: "private-turn-id".to_string(),
            round_id: "private-round-id".to_string(),
            round_group_id: None,
            round_index: 1,
            model_config_id: "model".to_string(),
            effective_model_name: "claude".to_string(),
        });
        subscriber.project(&AgenticEvent::ModelRoundAttemptSuperseded {
            session_id: "private-session-id".to_string(),
            turn_id: "private-turn-id".to_string(),
            round_id: "private-round-id".to_string(),
            diagnostic: crate::agentic::events::ModelRoundAttemptDiagnostic {
                attempt_id: "private-attempt".to_string(),
                attempt_index: 1,
                category: "network".to_string(),
                raw_error: Some("secret raw error".to_string()),
                tool_calls: vec![],
            },
        });
        subscriber.project(&AgenticEvent::ToolEvent {
            session_id: "private-session-id".to_string(),
            turn_id: "private-turn-id".to_string(),
            round_id: "private-round-id".to_string(),
            attempt_id: None,
            attempt_index: None,
            tool_event: ToolEventData::Rejected {
                identity: ToolEventIdentity::direct("private-tool", "Write"),
            },
        });
        subscriber.project(&AgenticEvent::DialogTurnCancelled {
            session_id: "private-session-id".to_string(),
            turn_id: "private-turn-id".to_string(),
        });

        let records = sink.records();
        let serialized = serde_json::to_string(&records).expect("serialize records");
        assert!(serialized.contains("blocked"));
        assert!(serialized.contains("rejected"));
        assert!(serialized.contains("cancelled"));
        assert!(serialized.contains("bitfun.deep_review.lifecycle"));
        assert!(!serialized.contains("secret raw error"));
        assert!(!serialized.contains("objective"));
        let goal_spans = records
            .iter()
            .filter_map(|record| match record {
                bitfun_observability::ValidatedRecord::Span(span)
                    if span.name() == "bitfun.goal.lifecycle" =>
                {
                    Some(span)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(goal_spans.len(), 2);
        assert!(goal_spans.iter().all(|span| span.links().len() == 1));
        assert!(records.iter().any(|record| {
            matches!(
                record,
                bitfun_observability::ValidatedRecord::Span(span)
                    if span.name() == "bitfun.deep_review.run"
            )
        }));
    }
}
