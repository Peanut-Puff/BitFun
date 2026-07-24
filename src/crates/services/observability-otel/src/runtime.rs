use crate::identity::InstallationIdentityStore;
use crate::pipeline::{OtelGeneration, PipelineDiagnostics};
use crate::settings::{validate_enabled_config, TelemetryRuntimeMetadata};
use crate::{NoTelemetrySecrets, TelemetryRuntimeError, TelemetrySecretProvider};
use bitfun_observability::config::TelemetryConfig;
use bitfun_observability::{
    PolicySnapshot, Telemetry, TelemetryControl, TelemetryDiagnostics, TelemetryLevel,
    TelemetrySink, ValidatedRecord,
};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TelemetryRuntimeHealth {
    pub enabled: bool,
    pub generation: u64,
    pub level: TelemetryLevel,
    pub facade: TelemetryDiagnostics,
    pub pipeline: PipelineDiagnostics,
    pub dropped_without_generation: u64,
    pub reconfigurations: u64,
    pub last_error_class: Option<&'static str>,
}

#[derive(Debug, Default)]
struct ReloadableOtelSink {
    generation: RwLock<Option<Arc<OtelGeneration>>>,
    retired: Mutex<PipelineDiagnostics>,
    dropped_without_generation: AtomicU64,
    generation_number: AtomicU64,
    reconfigurations: AtomicU64,
}

impl ReloadableOtelSink {
    fn install(&self, generation: Arc<OtelGeneration>) {
        let mut current = self
            .generation
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        debug_assert!(current.is_none());
        *current = Some(generation);
        self.generation_number.fetch_add(1, Ordering::AcqRel);
        self.reconfigurations.fetch_add(1, Ordering::Relaxed);
    }

    fn retire(&self, graceful: bool) -> Result<(), TelemetryRuntimeError> {
        let generation = self
            .generation
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        let Some(generation) = generation else {
            return Ok(());
        };
        if !graceful {
            generation.deactivate();
        }
        let result = generation.shutdown(graceful);
        merge_diagnostics(
            &mut self
                .retired
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            generation.diagnostics(),
        );
        result
    }

    fn force_flush(&self) -> Result<(), TelemetryRuntimeError> {
        let generation = self
            .generation
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        match generation {
            Some(generation) => generation.force_flush(),
            None => Ok(()),
        }
    }

    fn diagnostics(&self) -> (PipelineDiagnostics, u64, u64, u64, bool) {
        let mut pipeline = *self
            .retired
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let current = self
            .generation
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(generation) = &current {
            merge_diagnostics(&mut pipeline, generation.diagnostics());
        }
        (
            pipeline,
            self.dropped_without_generation.load(Ordering::Relaxed),
            self.generation_number.load(Ordering::Acquire),
            self.reconfigurations.load(Ordering::Relaxed),
            current.is_some(),
        )
    }
}

impl TelemetrySink for ReloadableOtelSink {
    fn emit(&self, record: ValidatedRecord) {
        let generation = self
            .generation
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(generation) = generation {
            generation.emit(record);
        } else {
            self.dropped_without_generation
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    fn discard_pending(&self) {
        if let Err(error) = self.retire(false) {
            log::warn!("Telemetry queue discard did not fully shut down: {error}");
        }
    }
}

struct TelemetryRuntimeInner {
    telemetry: Telemetry,
    control: TelemetryControl,
    sink: Arc<ReloadableOtelSink>,
    metadata: TelemetryRuntimeMetadata,
    identity: InstallationIdentityStore,
    secrets: Arc<dyn TelemetrySecretProvider>,
    instance_id: String,
    effective_config: Mutex<TelemetryConfig>,
    last_error_class: Mutex<Option<&'static str>>,
    shutdown: AtomicBool,
}

impl Drop for TelemetryRuntimeInner {
    fn drop(&mut self) {
        self.control.apply_policy(PolicySnapshot::default());
        let _ = self.sink.retire(false);
    }
}

#[derive(Clone)]
pub struct TelemetryRuntime {
    inner: Arc<TelemetryRuntimeInner>,
}

impl std::fmt::Debug for TelemetryRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TelemetryRuntime")
            .field("metadata", &self.inner.metadata)
            .field("health", &self.health())
            .finish_non_exhaustive()
    }
}

impl TelemetryRuntime {
    pub fn new(
        metadata: TelemetryRuntimeMetadata,
        secrets: Arc<dyn TelemetrySecretProvider>,
    ) -> Self {
        let sink = Arc::new(ReloadableOtelSink::default());
        let (telemetry, control) = Telemetry::build(PolicySnapshot::default(), sink.clone());
        let identity = InstallationIdentityStore::new(metadata.state_directory.join("telemetry"));
        Self {
            inner: Arc::new(TelemetryRuntimeInner {
                telemetry,
                control,
                sink,
                metadata,
                identity,
                secrets,
                instance_id: uuid::Uuid::new_v4().to_string(),
                effective_config: Mutex::new(TelemetryConfig::default()),
                last_error_class: Mutex::new(None),
                shutdown: AtomicBool::new(false),
            }),
        }
    }

    pub fn without_secrets(metadata: TelemetryRuntimeMetadata) -> Self {
        Self::new(metadata, Arc::new(NoTelemetrySecrets))
    }

    pub fn telemetry(&self) -> Telemetry {
        self.inner.telemetry.clone()
    }

    pub fn effective_config(&self) -> TelemetryConfig {
        self.inner
            .effective_config
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn apply_config(&self, config: TelemetryConfig) -> Result<(), TelemetryRuntimeError> {
        if self.inner.shutdown.load(Ordering::Acquire) {
            return Err(TelemetryRuntimeError::AlreadyShutdown);
        }
        if self.effective_config() == config {
            return Ok(());
        }

        if !config.is_enabled() {
            self.inner.control.apply_policy(PolicySnapshot::default());
            self.inner.sink.retire(false)?;
            self.set_effective_config(config);
            self.set_last_error(None);
            return Ok(());
        }

        let settings = match validate_enabled_config(&config, self.inner.secrets.as_ref()) {
            Ok(settings) => settings,
            Err(error) => {
                self.fail_closed("invalid_config");
                return Err(error);
            }
        };
        let scoped_installation_id = match self.inner.identity.scoped_id(&settings.audience) {
            Ok(identity) => identity,
            Err(error) => {
                self.fail_closed("identity");
                return Err(error);
            }
        };
        let generation = match OtelGeneration::build(
            &settings,
            &self.inner.metadata,
            scoped_installation_id,
            &self.inner.instance_id,
            config.signals,
        ) {
            Ok(generation) => Arc::new(generation),
            Err(error) => {
                self.fail_closed("exporter_setup");
                return Err(error);
            }
        };

        self.inner.control.apply_policy(PolicySnapshot::default());
        self.inner.sink.retire(false)?;
        self.inner.sink.install(generation);
        self.inner.control.apply_policy(config.policy_snapshot());
        self.set_effective_config(config);
        self.set_last_error(None);
        Ok(())
    }

    pub fn force_flush(&self) -> Result<(), TelemetryRuntimeError> {
        self.inner.sink.force_flush()
    }

    pub fn shutdown(&self) -> Result<(), TelemetryRuntimeError> {
        if self.inner.shutdown.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        self.inner.control.close_ingress_for_shutdown();
        let result = self.inner.sink.retire(true);
        self.set_effective_config(TelemetryConfig::default());
        if result.is_err() {
            self.set_last_error(Some("shutdown"));
        }
        result
    }

    pub fn reset_installation_identity(&self) -> Result<bool, TelemetryRuntimeError> {
        if self.inner.shutdown.load(Ordering::Acquire) {
            return Err(TelemetryRuntimeError::AlreadyShutdown);
        }
        self.inner.control.apply_policy(PolicySnapshot::default());
        self.inner.sink.retire(false)?;
        self.set_effective_config(TelemetryConfig::default());
        self.inner.identity.reset()
    }

    pub fn installation_identity_path(&self) -> std::path::PathBuf {
        self.inner.identity.identity_path()
    }

    pub fn health(&self) -> TelemetryRuntimeHealth {
        let (pipeline, dropped_without_generation, generation, reconfigurations, enabled) =
            self.inner.sink.diagnostics();
        TelemetryRuntimeHealth {
            enabled,
            generation,
            level: self.inner.control.policy_snapshot().level(),
            facade: self.inner.control.diagnostics(),
            pipeline,
            dropped_without_generation,
            reconfigurations,
            last_error_class: *self
                .inner
                .last_error_class
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        }
    }

    fn fail_closed(&self, error_class: &'static str) {
        self.inner.control.apply_policy(PolicySnapshot::default());
        if let Err(error) = self.inner.sink.retire(false) {
            log::warn!("Telemetry failed closed but cleanup was incomplete: {error}");
        }
        self.set_effective_config(TelemetryConfig::default());
        self.set_last_error(Some(error_class));
    }

    fn set_effective_config(&self, config: TelemetryConfig) {
        *self
            .inner
            .effective_config
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = config;
    }

    fn set_last_error(&self, error: Option<&'static str>) {
        *self
            .inner
            .last_error_class
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = error;
    }
}

fn merge_diagnostics(target: &mut PipelineDiagnostics, source: PipelineDiagnostics) {
    target.submitted = target.submitted.saturating_add(source.submitted);
    target.dropped_inactive = target
        .dropped_inactive
        .saturating_add(source.dropped_inactive);
    target.dropped_signal_disabled = target
        .dropped_signal_disabled
        .saturating_add(source.dropped_signal_disabled);
    target.exported_batches = target
        .exported_batches
        .saturating_add(source.exported_batches);
    target.export_failures = target
        .export_failures
        .saturating_add(source.export_failures);
    target.suppressed_batches = target
        .suppressed_batches
        .saturating_add(source.suppressed_batches);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DeploymentEnvironment, TelemetryEntrypoint};
    use bitfun_observability::config::TelemetryConfig;

    fn metadata(root: &std::path::Path) -> TelemetryRuntimeMetadata {
        TelemetryRuntimeMetadata::new("bitfun-test", "0.0.0", TelemetryEntrypoint::Cli, root)
            .with_environment(DeploymentEnvironment::Test)
    }

    #[test]
    fn off_runtime_creates_no_identity_or_generation() {
        let temporary = tempfile::tempdir().unwrap();
        let runtime = TelemetryRuntime::without_secrets(metadata(temporary.path()));

        runtime.apply_config(TelemetryConfig::default()).unwrap();

        assert!(!runtime.installation_identity_path().exists());
        assert!(!runtime.health().enabled);
        assert_eq!(runtime.health().level, TelemetryLevel::Off);
    }

    #[test]
    fn invalid_enabled_config_fails_closed_before_identity_creation() {
        let temporary = tempfile::tempdir().unwrap();
        let runtime = TelemetryRuntime::without_secrets(metadata(temporary.path()));
        let config = TelemetryConfig {
            level: TelemetryLevel::Basic,
            ..TelemetryConfig::default()
        };

        assert!(runtime.apply_config(config).is_err());

        assert!(!runtime.installation_identity_path().exists());
        assert_eq!(runtime.health().level, TelemetryLevel::Off);
        assert_eq!(runtime.health().last_error_class, Some("invalid_config"));
    }
}
