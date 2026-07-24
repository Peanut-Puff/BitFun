# BitFun OpenTelemetry service

This crate is the concrete service behind `bitfun-observability`. It owns the
OpenTelemetry SDK, OTLP HTTP/gRPC exporters, batching, pseudonymous installation
identity, runtime generations, and flush/shutdown behavior.

Product and business code must depend on the portable `Telemetry` facade. Only
application bootstrap and configuration owners should use this crate directly.
