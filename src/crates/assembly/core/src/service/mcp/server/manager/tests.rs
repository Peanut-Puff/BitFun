use bitfun_observability::domains::{start_mcp, McpOperation, McpStartFacts, McpTransport};
use bitfun_observability::{InMemorySink, PolicySnapshot, Telemetry, TelemetryLevel};
use bitfun_services_integrations::mcp::server::{
    compute_mcp_backoff_delay, detect_mcp_list_changed_kind, MCPListChangedKind,
};
use std::sync::Arc;
use std::time::Duration;

#[test]
fn backoff_delay_grows_exponentially_and_caps() {
    let base = Duration::from_secs(2);
    let max = Duration::from_secs(60);

    assert_eq!(
        compute_mcp_backoff_delay(base, max, 1),
        Duration::from_secs(2)
    );
    assert_eq!(
        compute_mcp_backoff_delay(base, max, 2),
        Duration::from_secs(4)
    );
    assert_eq!(
        compute_mcp_backoff_delay(base, max, 5),
        Duration::from_secs(32)
    );
    assert_eq!(
        compute_mcp_backoff_delay(base, max, 10),
        Duration::from_secs(60)
    );
}

#[test]
fn detect_list_changed_kind_supports_three_catalogs() {
    assert_eq!(
        detect_mcp_list_changed_kind("notifications/tools/list_changed"),
        Some(MCPListChangedKind::Tools)
    );
    assert_eq!(
        detect_mcp_list_changed_kind("notifications/prompts/list_changed"),
        Some(MCPListChangedKind::Prompts)
    );
    assert_eq!(
        detect_mcp_list_changed_kind("notifications/resources/list_changed"),
        Some(MCPListChangedKind::Resources)
    );
    assert_eq!(detect_mcp_list_changed_kind("notifications/unknown"), None);
}

#[test]
fn ephemeral_retirement_waits_for_in_flight_connection_users_but_is_bounded() {
    let grace = Duration::from_secs(30);
    assert!(super::should_finish_ephemeral_retirement(
        2,
        Duration::ZERO,
        grace
    ));
    assert!(!super::should_finish_ephemeral_retirement(
        3,
        Duration::from_secs(10),
        grace
    ));
    assert!(super::should_finish_ephemeral_retirement(3, grace, grace));
}

#[test]
fn retired_external_start_cannot_publish_after_handshake() {
    assert!(super::external_start_publication_allowed(false, true));
    assert!(super::external_start_publication_allowed(true, false));
    assert!(!super::external_start_publication_allowed(true, true));
}

#[test]
fn superseded_external_start_token_cannot_clean_up_current_instance() {
    let first = std::sync::Arc::new(());
    let current = std::sync::Arc::new(());

    assert!(super::external_start_token_is_current(Some(&first), &first));
    assert!(!super::external_start_token_is_current(
        Some(&current),
        &first
    ));
    assert!(!super::external_start_token_is_current(None, &first));
}

#[test]
fn mcp_terminal_facts_keep_safe_outcomes_and_retry_buckets() {
    let sink = Arc::new(InMemorySink::default());
    let (telemetry, _) = Telemetry::build(
        PolicySnapshot::new(TelemetryLevel::Diagnostic)
            .with_trace_sample_ratio(1.0)
            .with_success_log_sample_ratio(1.0),
        sink.clone(),
    );

    let timeout = start_mcp(
        &telemetry,
        McpStartFacts {
            operation: McpOperation::Connect,
            transport: McpTransport::StreamableHttp,
        },
        None,
    );
    let timeout_result: super::BitFunResult<()> = Err(super::BitFunError::Timeout(
        "private endpoint timeout".to_string(),
    ));
    super::finish_mcp_result(timeout, &timeout_result, 2);

    let cancelled = start_mcp(
        &telemetry,
        McpStartFacts {
            operation: McpOperation::CallTool,
            transport: McpTransport::Stdio,
        },
        None,
    );
    let cancelled_result: super::BitFunResult<()> = Err(super::BitFunError::Cancelled(
        "private cancellation detail".to_string(),
    ));
    super::finish_mcp_result(cancelled, &cancelled_result, 3);

    let encoded = serde_json::to_string(&sink.records()).expect("serialize records");
    assert!(encoded.contains("timeout"));
    assert!(encoded.contains("cancelled"));
    assert!(encoded.contains("\"2\""));
    assert!(encoded.contains("3_plus"));
    assert!(!encoded.contains("private endpoint"));
    assert!(!encoded.contains("private cancellation"));
}
