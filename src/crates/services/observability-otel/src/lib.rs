//! Concrete OpenTelemetry runtime for BitFun's portable telemetry facade.
//!
//! Business owners depend on `bitfun-observability`; application bootstrap and
//! configuration owners use this crate to install an OTLP runtime generation.

mod error;
mod identity;
mod pipeline;
mod runtime;
mod secrets;
mod settings;

pub use error::TelemetryRuntimeError;
pub use identity::InstallationIdentityStore;
pub use pipeline::PipelineDiagnostics;
pub use runtime::{TelemetryRuntime, TelemetryRuntimeHealth};
pub use secrets::{
    EnvironmentTelemetrySecrets, NoTelemetrySecrets, OtlpHeaders, TelemetrySecretProvider,
};
pub use settings::{
    deployment_config_from_env, DeploymentEnvironment, TelemetryEntrypoint,
    TelemetryRuntimeMetadata,
};
