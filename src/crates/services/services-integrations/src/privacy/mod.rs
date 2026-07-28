use bitfun_product_domains::privacy::{
    AcceptPrivacyRequest, PrivacyChangeType, PrivacyConsentRecord, PrivacyEffectiveMode,
    PrivacyError, PrivacyLifecycleState, PrivacyPolicyView, PrivacyStatus,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

const POLICY_VERSION: &str = "2026.07.1-dev-placeholder";
const CONSENT_VERSION: &str = "dev-placeholder-1";
const EFFECTIVE_AT: &str = "2026-07-22T00:00:00Z";
const UPDATED_AT: &str = "2026-07-22T00:00:00Z";
const CHANGE_TYPE: PrivacyChangeType = PrivacyChangeType::Material;
const LEGAL_CONTENT_SENTINEL: &str = "LEGAL_CONTENT_REQUIRED";

const ZH_CN_CONTENT: &str = include_str!("assets/zh-CN.md");
const EN_US_CONTENT: &str = include_str!("assets/en-US.md");
const ZH_CN_SHA256: &str = "9164815a22b2b2021039a19ed6e92556ce6ea44e42dd0103869b7c0887ae48bb";
const EN_US_SHA256: &str = "71c9914ad977ff12fa31a5e228192d806b3b3d366498100bff615739d9b4c451";
const ACCEPTED_POLICY_HASHES: &[(&str, &str, &str)] = &[
    (POLICY_VERSION, "zh-CN", ZH_CN_SHA256),
    (POLICY_VERSION, "en-US", EN_US_SHA256),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredPrivacyMode {
    Full,
    PrivacyNotAccepted,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrivacyStateFile {
    #[serde(default)]
    mode: Option<StoredPrivacyMode>,
    #[serde(default)]
    consent: Option<PrivacyConsentRecord>,
    #[serde(default)]
    viewed_policy_version: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct BuiltinDocument {
    locale: &'static str,
    content: &'static str,
    expected_sha256: &'static str,
}

#[derive(Debug)]
pub struct PrivacyCollectionPolicy {
    collection_allowed: AtomicBool,
    #[cfg(test)]
    fail_full_application: AtomicBool,
}

impl PrivacyCollectionPolicy {
    pub fn new(collection_allowed: bool) -> Self {
        Self {
            collection_allowed: AtomicBool::new(collection_allowed),
            #[cfg(test)]
            fail_full_application: AtomicBool::new(false),
        }
    }

    pub fn apply(&self, mode: PrivacyEffectiveMode) -> Result<(), PrivacyError> {
        #[cfg(test)]
        if mode == PrivacyEffectiveMode::Full && self.fail_full_application.load(Ordering::SeqCst) {
            self.collection_allowed.store(false, Ordering::SeqCst);
            return Err(PrivacyError::new(
                "PRIVACY_POLICY_APPLY_FAILED",
                "The full collection policy could not be applied",
            ));
        }

        self.collection_allowed
            .store(mode == PrivacyEffectiveMode::Full, Ordering::SeqCst);
        Ok(())
    }

    pub fn effective_mode(&self) -> PrivacyEffectiveMode {
        if self.collection_allowed.load(Ordering::SeqCst) {
            PrivacyEffectiveMode::Full
        } else {
            PrivacyEffectiveMode::PrivacyNotAccepted
        }
    }

    pub fn collection_allowed(&self) -> bool {
        self.collection_allowed.load(Ordering::SeqCst)
    }

    #[cfg(test)]
    fn set_fail_full_application(&self, fail: bool) {
        self.fail_full_application.store(fail, Ordering::SeqCst);
    }
}

pub struct PrivacyService {
    state_path: PathBuf,
    initial_locale: String,
    resources_valid: bool,
    release_ready: bool,
}

impl PrivacyService {
    pub fn new(storage_dir: PathBuf, initial_locale: &str) -> Self {
        let documents = builtin_documents();
        let resources_valid = documents.iter().all(|document| {
            sha256(document.content.as_bytes()) == document.expected_sha256
                && !document.content.trim().is_empty()
        });
        let legal_content_ready = documents
            .iter()
            .all(|document| !document.content.contains(LEGAL_CONTENT_SENTINEL));
        Self {
            state_path: storage_dir.join("privacy-state.json"),
            initial_locale: normalize_locale(initial_locale).to_string(),
            resources_valid,
            release_ready: resources_valid && (legal_content_ready || cfg!(debug_assertions)),
        }
    }

    pub async fn initialize(&self, app_version: &str) -> Result<PrivacyStatus, PrivacyError> {
        self.status(&self.initial_locale, app_version).await
    }

    pub async fn status(
        &self,
        locale: &str,
        _app_version: &str,
    ) -> Result<PrivacyStatus, PrivacyError> {
        if !self.resources_valid {
            return Ok(PrivacyStatus {
                enabled: true,
                lifecycle_state: PrivacyLifecycleState::ResourceError,
                effective_mode: PrivacyEffectiveMode::PrivacyNotAccepted,
                release_ready: false,
                has_unread_update: false,
                policy: None,
                consent: None,
                configuration_error: Some("BUILT_IN_POLICY_INVALID".to_string()),
            });
        }

        let document = self.document(locale)?;
        let state = self.load_state().await;
        let consent_valid = state
            .consent
            .as_ref()
            .is_some_and(|consent| self.consent_is_valid(consent));
        let lifecycle_state = match state.mode {
            Some(StoredPrivacyMode::Full) if consent_valid => PrivacyLifecycleState::Full,
            Some(StoredPrivacyMode::PrivacyNotAccepted) => {
                PrivacyLifecycleState::PrivacyNotAccepted
            }
            _ => PrivacyLifecycleState::ChoiceRequired,
        };
        let effective_mode = if lifecycle_state == PrivacyLifecycleState::Full {
            PrivacyEffectiveMode::Full
        } else {
            PrivacyEffectiveMode::PrivacyNotAccepted
        };
        let has_unread_update = lifecycle_state == PrivacyLifecycleState::Full
            && state
                .consent
                .as_ref()
                .is_some_and(|consent| consent.accepted_policy_version != POLICY_VERSION)
            && state.viewed_policy_version.as_deref() != Some(POLICY_VERSION);

        Ok(PrivacyStatus {
            enabled: true,
            lifecycle_state,
            effective_mode,
            release_ready: self.release_ready,
            has_unread_update,
            policy: Some(policy_view(document)),
            consent: state
                .consent
                .filter(|consent| self.consent_is_valid(consent)),
            configuration_error: (!self.release_ready)
                .then(|| "LEGAL_CONTENT_REQUIRED".to_string()),
        })
    }

    pub async fn accept(
        &self,
        request: &AcceptPrivacyRequest,
        app_version: &str,
    ) -> Result<PrivacyStatus, PrivacyError> {
        if !self.release_ready {
            return Err(PrivacyError::new(
                "PRIVACY_RELEASE_BLOCKED",
                "Bundled privacy resources are not release ready",
            ));
        }
        let document = self.document(&request.locale)?;
        let expected_hash = sha256(document.content.as_bytes());
        if request.policy_version != POLICY_VERSION
            || request.consent_version != CONSENT_VERSION
            || request.document_sha256 != expected_hash
        {
            return Err(PrivacyError::new(
                "PRIVACY_POLICY_MISMATCH",
                "The displayed privacy policy no longer matches the bundled policy",
            ));
        }

        let consent = PrivacyConsentRecord {
            consent_version: CONSENT_VERSION.to_string(),
            accepted_policy_version: POLICY_VERSION.to_string(),
            accepted_document_sha256: expected_hash,
            accepted_at: Utc::now().to_rfc3339(),
            locale: document.locale.to_string(),
            app_version: app_version.to_string(),
        };
        self.store_state(&PrivacyStateFile {
            mode: Some(StoredPrivacyMode::Full),
            consent: Some(consent),
            viewed_policy_version: Some(POLICY_VERSION.to_string()),
        })
        .await?;
        self.status(document.locale, app_version).await
    }

    pub async fn enter_not_accepted(
        &self,
        locale: &str,
        app_version: &str,
    ) -> Result<PrivacyStatus, PrivacyError> {
        let state = self.load_state().await;
        self.store_state(&PrivacyStateFile {
            mode: Some(StoredPrivacyMode::PrivacyNotAccepted),
            consent: None,
            viewed_policy_version: state.viewed_policy_version,
        })
        .await?;
        self.status(locale, app_version).await
    }

    pub async fn mark_viewed(
        &self,
        policy_version: &str,
        app_version: &str,
        locale: &str,
    ) -> Result<PrivacyStatus, PrivacyError> {
        if policy_version != POLICY_VERSION {
            return Err(PrivacyError::new(
                "PRIVACY_POLICY_MISMATCH",
                "Policy version does not match the bundled policy",
            ));
        }
        let mut state = self.load_state().await;
        state.viewed_policy_version = Some(POLICY_VERSION.to_string());
        self.store_state(&state).await?;
        self.status(locale, app_version).await
    }

    pub async fn can_apply_full_mode(&self) -> Result<bool, PrivacyError> {
        let status = self.status(&self.initial_locale, "policy-check").await?;
        Ok(status.lifecycle_state == PrivacyLifecycleState::Full)
    }

    fn document(&self, locale: &str) -> Result<BuiltinDocument, PrivacyError> {
        if !self.resources_valid {
            return Err(PrivacyError::new(
                "BUILT_IN_POLICY_INVALID",
                "A bundled privacy document failed integrity validation",
            ));
        }
        let locale = normalize_locale(locale);
        builtin_documents()
            .into_iter()
            .find(|document| document.locale == locale)
            .ok_or_else(|| {
                PrivacyError::new(
                    "PRIVACY_LOCALE_UNAVAILABLE",
                    format!("Bundled privacy document is unavailable for {locale}"),
                )
            })
    }

    fn consent_is_valid(&self, consent: &PrivacyConsentRecord) -> bool {
        if consent.consent_version != CONSENT_VERSION
            || consent.accepted_policy_version.trim().is_empty()
            || consent.app_version.trim().is_empty()
            || chrono::DateTime::parse_from_rfc3339(&consent.accepted_at).is_err()
            || !is_sha256(&consent.accepted_document_sha256)
        {
            return false;
        }
        let Ok(document) = self.document(&consent.locale) else {
            return false;
        };
        accepted_policy_hash(&consent.accepted_policy_version, document.locale)
            .is_some_and(|expected| consent.accepted_document_sha256 == expected)
    }

    async fn load_state(&self) -> PrivacyStateFile {
        let Ok(bytes) = tokio::fs::read(&self.state_path).await else {
            return PrivacyStateFile::default();
        };
        serde_json::from_slice(&bytes).unwrap_or_default()
    }

    async fn store_state(&self, state: &PrivacyStateFile) -> Result<(), PrivacyError> {
        let parent = self.state_path.parent().ok_or_else(|| {
            PrivacyError::new("PRIVACY_STORAGE_ERROR", "Privacy storage path is invalid")
        })?;
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(storage_error)?;
        let body = serde_json::to_vec_pretty(state).map_err(|error| {
            PrivacyError::new(
                "PRIVACY_STORAGE_ERROR",
                format!("Encode privacy state: {error}"),
            )
        })?;
        let temporary = self.state_path.with_extension("json.tmp");
        tokio::fs::write(&temporary, body)
            .await
            .map_err(storage_error)?;
        tokio::fs::rename(&temporary, &self.state_path)
            .await
            .map_err(storage_error)?;
        Ok(())
    }
}

fn builtin_documents() -> [BuiltinDocument; 2] {
    [
        BuiltinDocument {
            locale: "zh-CN",
            content: ZH_CN_CONTENT,
            expected_sha256: ZH_CN_SHA256,
        },
        BuiltinDocument {
            locale: "en-US",
            content: EN_US_CONTENT,
            expected_sha256: EN_US_SHA256,
        },
    ]
}

fn policy_view(document: BuiltinDocument) -> PrivacyPolicyView {
    PrivacyPolicyView {
        policy_version: POLICY_VERSION.to_string(),
        consent_version: CONSENT_VERSION.to_string(),
        change_type: CHANGE_TYPE,
        effective_at: EFFECTIVE_AT.to_string(),
        updated_at: UPDATED_AT.to_string(),
        locale: document.locale.to_string(),
        document_sha256: sha256(document.content.as_bytes()),
        content: document.content.to_string(),
    }
}

fn normalize_locale(locale: &str) -> &'static str {
    let normalized = locale.trim().to_ascii_lowercase().replace('_', "-");
    if normalized == "zh" || normalized.starts_with("zh-") {
        "zh-CN"
    } else {
        "en-US"
    }
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn accepted_policy_hash(policy_version: &str, locale: &str) -> Option<&'static str> {
    ACCEPTED_POLICY_HASHES
        .iter()
        .find(|(version, accepted_locale, _)| {
            *version == policy_version && *accepted_locale == locale
        })
        .map(|(_, _, hash)| *hash)
}

fn storage_error(error: std::io::Error) -> PrivacyError {
    PrivacyError::new(
        "PRIVACY_STORAGE_ERROR",
        format!("Persist privacy state: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::{PrivacyCollectionPolicy, PrivacyService, normalize_locale};
    use bitfun_product_domains::privacy::{
        AcceptPrivacyRequest, PrivacyEffectiveMode, PrivacyLifecycleState,
    };

    fn temporary_directory(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "bitfun-privacy-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    async fn accept(service: &PrivacyService) {
        let initial = service.initialize("1.2.3").await.unwrap();
        let policy = initial.policy.unwrap();
        service
            .accept(
                &AcceptPrivacyRequest {
                    policy_version: policy.policy_version,
                    consent_version: policy.consent_version,
                    document_sha256: policy.document_sha256,
                    locale: policy.locale,
                },
                "1.2.3",
            )
            .await
            .unwrap();
    }

    #[test]
    fn resolves_traditional_chinese_to_simplified_policy() {
        assert_eq!(normalize_locale("zh-Hant-HK"), "zh-CN");
        assert_eq!(normalize_locale("zh-TW"), "zh-CN");
        assert_eq!(normalize_locale("zh-HK"), "zh-CN");
        assert_eq!(normalize_locale("en-GB"), "en-US");
    }

    #[tokio::test]
    async fn first_start_requires_a_choice_without_blocking_local_initialization() {
        let directory = temporary_directory("first-start");
        let service = PrivacyService::new(directory.clone(), "zh-TW");
        let status = service.initialize("test").await.unwrap();
        assert_eq!(
            status.lifecycle_state,
            PrivacyLifecycleState::ChoiceRequired
        );
        assert_eq!(
            status.effective_mode,
            PrivacyEffectiveMode::PrivacyNotAccepted
        );
        assert_eq!(status.policy.unwrap().locale, "zh-CN");
        let _ = tokio::fs::remove_dir_all(directory).await;
    }

    #[tokio::test]
    async fn not_accepted_survives_a_cold_start() {
        let directory = temporary_directory("not-accepted");
        let service = PrivacyService::new(directory.clone(), "en-US");
        service.enter_not_accepted("en-US", "1.2.3").await.unwrap();
        let restarted = PrivacyService::new(directory.clone(), "en-US");
        assert_eq!(
            restarted.initialize("1.2.3").await.unwrap().lifecycle_state,
            PrivacyLifecycleState::PrivacyNotAccepted
        );
        let _ = tokio::fs::remove_dir_all(directory).await;
    }

    #[tokio::test]
    async fn accept_persists_full_mode_atomically() {
        let directory = temporary_directory("accept");
        let service = PrivacyService::new(directory.clone(), "en-US");
        accept(&service).await;
        let status = PrivacyService::new(directory.clone(), "en-US")
            .initialize("1.2.3")
            .await
            .unwrap();
        assert_eq!(status.lifecycle_state, PrivacyLifecycleState::Full);
        assert!(status.consent.is_some());
        assert!(!directory.join("privacy-state.tmp").exists());
        let _ = tokio::fs::remove_dir_all(directory).await;
    }

    #[test]
    fn collection_policy_fails_closed() {
        let policy = PrivacyCollectionPolicy::new(false);
        policy.set_fail_full_application(true);
        assert!(policy.apply(PrivacyEffectiveMode::Full).is_err());
        assert!(!policy.collection_allowed());
        policy
            .apply(PrivacyEffectiveMode::PrivacyNotAccepted)
            .unwrap();
        assert!(!policy.collection_allowed());
    }
}
