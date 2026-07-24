use crate::TelemetryRuntimeError;
use std::collections::HashMap;

pub type OtlpHeaders = HashMap<String, String>;

pub trait TelemetrySecretProvider: Send + Sync + 'static {
    fn resolve_headers(&self, secret_ref: &str) -> Result<OtlpHeaders, TelemetryRuntimeError>;
}

#[derive(Debug, Default)]
pub struct NoTelemetrySecrets;

impl TelemetrySecretProvider for NoTelemetrySecrets {
    fn resolve_headers(&self, _secret_ref: &str) -> Result<OtlpHeaders, TelemetryRuntimeError> {
        Err(TelemetryRuntimeError::Secret(
            "no credential provider is installed",
        ))
    }
}

/// Deployment-only secret resolver. The referenced environment variable must
/// contain a JSON object whose values are strings. The values are never logged.
#[derive(Debug, Default)]
pub struct EnvironmentTelemetrySecrets;

impl TelemetrySecretProvider for EnvironmentTelemetrySecrets {
    fn resolve_headers(&self, secret_ref: &str) -> Result<OtlpHeaders, TelemetryRuntimeError> {
        let variable = secret_ref
            .strip_prefix("env:")
            .filter(|name| valid_environment_name(name))
            .ok_or(TelemetryRuntimeError::Secret(
                "environment references must use env:VARIABLE_NAME",
            ))?;
        let value = std::env::var(variable)
            .map_err(|_| TelemetryRuntimeError::Secret("referenced secret is unavailable"))?;
        let parsed: serde_json::Value = serde_json::from_str(&value)
            .map_err(|_| TelemetryRuntimeError::Secret("secret must be a JSON object"))?;
        let object = parsed.as_object().ok_or(TelemetryRuntimeError::Secret(
            "secret must be a JSON object",
        ))?;

        if object.len() > 32 {
            return Err(TelemetryRuntimeError::Secret(
                "secret contains too many headers",
            ));
        }
        object
            .iter()
            .map(|(name, value)| {
                let value = value.as_str().ok_or(TelemetryRuntimeError::Secret(
                    "secret header values must be strings",
                ))?;
                Ok((name.clone(), value.to_string()))
            })
            .collect()
    }
}

fn valid_environment_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_reference_names_are_narrowly_scoped() {
        assert!(valid_environment_name("BITFUN_OTLP_HEADERS"));
        assert!(!valid_environment_name("BitFunHeaders"));
        assert!(!valid_environment_name("../TOKEN"));
        assert!(!valid_environment_name(""));
    }
}
