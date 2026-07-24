# BitFun Observability Contracts

`bitfun-observability` is the portable boundary between product facts and any
telemetry runtime. It owns typed domain facts, the static descriptor registry,
privacy validation, policy snapshots, safe intermediate records, and the sink
contract. It does not own an OpenTelemetry SDK, OTLP transport, queues,
credentials, installation identity, application bootstrap, or business state.

## Stable Boundary

Later delivery increments build on these contracts:

1. A concrete service implements `TelemetrySink` and maps `ValidatedRecord` to
   its SDK without exposing SDK types to this crate.
2. Product assembly injects that sink, retains `TelemetryControl`, and manages
   its runtime lifecycle. Business owners receive only `Telemetry`.
3. Business owners call functions in `domains` with typed start and completion
   facts. They do not submit attribute names, event names, log bodies, JSON, or
   raw errors.
4. New domains add typed facts and static descriptors. They do not change the
   sink trait, existing domain APIs, or privacy validation algorithm.

Descriptors also carry protocol-neutral metric units, sampling, rate-limit,
retention, cardinality, owner, and consumer metadata. A later SDK adapter reads
that metadata instead of maintaining an exporter-specific schema table.

`SpanContext` is an in-process value object in this increment. A later trusted
transport adapter may add W3C Trace Context parsing and formatting around it;
the wire format is intentionally not part of this crate yet.

## Safety Invariants

- `TelemetryLevel::Off` emits nothing.
- A policy change advances a revision. Observations from an older revision are
  discarded instead of crossing a consent boundary.
- Reducing authorization asks the sink to discard pending records before the
  new policy is published.
- Only descriptor-listed fields and enum values can reach a sink.
- Dynamic strings are limited to a validated product version. There is no field
  for prompts, responses, tool payloads, paths, user/device identity, endpoints,
  credentials, raw errors, or stack traces.
- Metric labels are bounded enums or booleans and have a contract-tested
  combination budget.
- Sink failures must remain outside product control flow. Network sinks should
  use a bounded non-blocking queue.

Run the focused contract suite with:

```bash
cargo test -p bitfun-observability
```
