# Backend Logging Specification

This specification covers two separate outputs:

- **Local diagnostic logs** produced through `log` or `tracing` and written to
  the configured console or local files.
- **Remote OTel Logs** produced only from typed facts in
  `bitfun-observability` and sent by an explicitly enabled telemetry runtime.

They are not two destinations for the same record. There must be no generic
bridge from local logger targets to an OTLP Logs exporter.

## Rules

1. **Use English only** - All log messages must be in English
2. **No emojis** - Do not use emojis in log messages
3. **Structured logging** - Include relevant context and metadata in log messages using formatted strings with key-value information
4. **Avoid verbose logging** - Keep log statements concise and meaningful, avoid excessive logging in normal operation paths

## Log Levels

| Level | Value | Usage |
|-------|-------|-------|
| TRACE | 0 | Verbose diagnostic info, performance-sensitive paths |
| DEBUG | 1 | Development debugging, internal state |
| INFO | 2 | General operational info (default in dev) |
| WARN | 3 | Potential issues, degraded functionality (default in prod) |
| ERROR | 4 | Failures, exceptions, requires attention |

## Guidelines

1. Import log macros at the top of the file: `use log::{info, debug, warn, error, trace};`
2. Include relevant context in log messages using formatted strings: `info!("Registered {} adapter for session: {}", adapter_type, session_id)`
3. Pass Error objects using Display formatting: `error!("Failed to emit event for session {}: {}", session_id, e)`
4. Avoid logging sensitive data (tokens, passwords, PII, API keys)
5. Avoid excessive logging in hot paths (loops, frequent callbacks, tight loops)
6. Use TRACE for expensive computations that may impact performance
7. For local diagnostics, include only the identifiers and context required to
   investigate the issue; do not add `user_id` or other identity by default
8. Use appropriate log levels - reserve ERROR for actual failures, not expected error conditions
9. Keep log messages concise and actionable - focus on what happened and why it matters
10. Use conditional logging for expensive operations: `if log::log_enabled!(log::Level::Debug) { ... }`

## Timing And Duration Fields

Use shared timing helpers from `bitfun_core::util::timing` when recording internal durations.

```rust
use bitfun_core::util::{elapsed_ms_u64, TimingCollector};
use std::time::Instant;

let started_at = Instant::now();
let duration_ms = elapsed_ms_u64(started_at);
debug!("Git status completed: repo_path={}, duration_ms={}", repo_path, duration_ms);
```

Rules:

1. Prefer `elapsed_ms`, `elapsed_ms_u64`, and `TimingCollector` over repeated `Instant::now()` plus `elapsed().as_millis()` formatting
2. Use `duration_ms` for Rust diagnostic log keys
3. Preserve existing protocol and model field names such as `duration_ms`, `execution_time_ms`, or `response_time_ms` when they are part of events, API responses, or persisted state
4. Avoid introducing timing logs into tight loops or high-frequency runtime paths unless the diagnostic value clearly justifies it

## Local And Remote Boundary

Local logs may contain operational details needed for user-initiated diagnosis,
subject to the application's local logging and sensitive-diagnostics settings.
They must still avoid credentials, authentication material, and unnecessary
personal data. `ModelExchangeTraceSink`, terminal output, commands, paths, raw
errors, and stack traces are local-only sources and must never be attached to a
remote telemetry sink.

Remote OTel Logs use the standard OpenTelemetry `LogRecord` data model, but a
business owner does not construct a `LogRecord` directly. It submits a typed
start/completion fact through `bitfun_observability::domains`; the descriptor
registry supplies the versioned event name and static body, and the privacy gate
validates every attribute before the sink sees it.

Remote records must not contain:

- inference requests or responses, prompts, thinking, or model output;
- tool arguments or results, commands, terminal input/output, or MCP payloads;
- file content, diffs, paths, working directories, repository URLs, or branches;
- names, email addresses, account or organization identifiers, machine names,
  IP/MAC addresses, or other user/device identity;
- endpoints, headers, credentials, environment variables, raw error messages,
  provider bodies, or stack traces;
- extension names, source text, configuration payloads, or return values.

Remote log rules:

1. Event names are stable `bitfun.*` descriptor names; do not derive a name from
   success/failure or user input.
2. Bodies are fixed English descriptor text, not formatted messages.
3. Attributes are registered enums, booleans, counts, durations, ratios, or a
   validated product version. Custom names are classified as `custom`.
4. `ERROR` represents a failed operation, `WARN` a rejected/blocked/degraded
   operation, and `INFO` a normal lifecycle result. `DEBUG` is diagnostic-only.
5. `ERROR` and `WARN` are not probability-sampled. High-frequency successful
   `INFO` records may use the descriptor sampling class; every level remains
   subject to bounded queues and rate limits.
6. Trace/span correlation is added only from an active sampled context. Session,
   turn, goal, installation, or business IDs are not log attributes.
7. Adding a remote event requires typed facts, a static descriptor, a named
   owner and consumer, a cardinality budget, and privacy contract coverage.

## Remote OTel Log Catalog

Remote OTel Logs report operation results that are useful for product
reliability, latency, and failure analysis. They do not report arbitrary local
messages. A typed operation emits one completion log when its descriptor and
telemetry policy allow it; the same typed facts also produce the matching trace
and metrics.

| Scope | OTel event names | Sent result |
|---|---|---|
| Application and agent lifecycle | `bitfun.app.lifecycle`, `bitfun.session.lifecycle`, `bitfun.agent.turn`, `bitfun.agent.round`, `bitfun.inference.request` | Entrypoint or bounded operation class, outcome, duration, safe error type, and registered count/bucket fields |
| Execution controls | `bitfun.tool.execution`, `bitfun.permission.decision`, `bitfun.context.operation` | Bounded tool category, permission decision class, compaction class, outcome, duration, and safe error type |
| Product workflows | `bitfun.goal.lifecycle`, `bitfun.deep_review.lifecycle` | Bounded operation/stage class, outcome, duration, and aggregate counts; never goal text, review content, or findings |
| Extensions and protocols | `bitfun.extension.invocation`, `bitfun.extension.lifecycle`, `bitfun.protocol.operation`, `bitfun.remote.operation` | Extension class, protocol/transport class, attempt bucket, outcome, duration, and safe error type; never names, payloads, endpoints, or paths |

Successful high-frequency events may be sampled. Failed, timed-out, cancelled,
rejected, blocked, and degraded terminal results are retained by the sampling
policy, subject only to the bounded queue and rate limits. Do not create remote
OTel Logs for ordinary cache hits, polling ticks, text chunks, terminal output,
file-watch callbacks, UI renders, or debug probes.

## Retention And Access

The client does not create a second on-disk copy of remote OTel Logs. Before
export, records exist only in the telemetry runtime's bounded memory queue. When
telemetry is disabled, pending records are discarded and no new remote records
are accepted.

The receiving deployment must apply these defaults unless a stricter product or
legal policy is configured:

| Data | Maximum raw retention | Access |
|---|---|---|
| Basic OTel Logs | 30 days | Named telemetry/reliability operators |
| Diagnostic OTel Logs | 7 days | Time-bound diagnostic access with audit records |
| Aggregates derived from logs | 90 days | Product and reliability owners; no raw attributes added during aggregation |

Backend deletion must remove expired raw records and honor installation-level
deletion requests where the deployment supports them. Export endpoints,
credentials, access policy, and retention are deployment configuration, not
business-event attributes.

Local log retention remains an app-host concern. Use bounded rotation and avoid
indefinite retention. Local support bundles require an explicit user action and
must not be uploaded through the telemetry exporter.

## Adding Or Changing Logs

For a local diagnostic log, first confirm that the message is needed to diagnose
a concrete condition. Choose the lowest useful level, keep the message static,
and include only the minimum safe context. Never log request/response bodies,
prompts, tool or MCP payloads, terminal content, credentials, or environment
values, even at `TRACE`.

For a remote OTel Log, do not call an exporter or construct a `LogRecord` in the
business module. Add or reuse a typed fact in `bitfun-observability::domains`,
register its static schema and owner, and finish it at the operation's real
terminal state. Cover success and every terminal state the owner can actually
produce, including timeout and cancellation where applicable. Tests must prove
the stable event name, bounded fields, trace correlation when present, and
absence of raw identifiers, paths, payloads, and error messages.
