use bitfun_product_domains::privacy::{
    AcceptPrivacyRequest, ApplyPrivacyCollectionPolicyRequest, EnterPrivacyNotAcceptedRequest,
    GetPrivacyStatusRequest, InitializePrivacyRequest, MarkPrivacyViewedRequest,
    PrivacyEffectiveMode, PrivacyError, PrivacyStatus,
};
use bitfun_services_integrations::privacy::{PrivacyCollectionPolicy, PrivacyService};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::State;
use tokio::sync::Mutex;

pub struct PrivacyServiceState {
    service: Option<Mutex<PrivacyService>>,
    collection_policy: Arc<PrivacyCollectionPolicy>,
    full_mode_active: AtomicBool,
}

static COLLECTION_POLICY: OnceLock<Arc<PrivacyCollectionPolicy>> = OnceLock::new();
fn shared_collection_policy(initially_allowed: bool) -> Arc<PrivacyCollectionPolicy> {
    COLLECTION_POLICY
        .get_or_init(|| Arc::new(PrivacyCollectionPolicy::new(initially_allowed)))
        .clone()
}

pub fn require_collection_allowed() -> Result<(), String> {
    #[cfg(target_env = "ohos")]
    if !COLLECTION_POLICY
        .get_or_init(|| Arc::new(PrivacyCollectionPolicy::new(false)))
        .collection_allowed()
    {
        return Err(
            "PRIVACY_CONSENT_REQUIRED: This network capability is disabled in the current privacy mode"
                .to_string(),
        );
    }
    Ok(())
}

impl PrivacyServiceState {
    pub fn enabled(storage_dir: PathBuf, locale: &str) -> Self {
        Self {
            service: Some(Mutex::new(PrivacyService::new(storage_dir, locale))),
            collection_policy: shared_collection_policy(false),
            full_mode_active: AtomicBool::new(false),
        }
    }

    pub fn disabled() -> Self {
        Self {
            service: None,
            collection_policy: shared_collection_policy(true),
            full_mode_active: AtomicBool::new(true),
        }
    }

    pub fn collection_allowed(&self) -> bool {
        self.collection_policy.collection_allowed()
    }

    fn enter_full_mode(&self) -> Result<bool, PrivacyError> {
        self.collection_policy.apply(PrivacyEffectiveMode::Full)?;
        Ok(!self.full_mode_active.swap(true, Ordering::SeqCst))
    }

    fn enter_not_accepted_mode(&self) -> Result<(), PrivacyError> {
        self.collection_policy
            .apply(PrivacyEffectiveMode::PrivacyNotAccepted)?;
        self.full_mode_active.store(false, Ordering::SeqCst);
        Ok(())
    }

    async fn with_service<T>(
        &self,
        operation: impl for<'a> FnOnce(
            &'a PrivacyService,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<T, PrivacyError>> + Send + 'a>,
        >,
    ) -> Result<T, PrivacyError> {
        let service = self.service.as_ref().ok_or_else(|| {
            PrivacyError::new(
                "PRIVACY_SERVICE_UNAVAILABLE",
                "Privacy service is unavailable",
            )
        })?;
        let guard = service.lock().await;
        operation(&guard).await
    }

    async fn status_with_effective_mode(
        &self,
        mut status: PrivacyStatus,
    ) -> Result<PrivacyStatus, PrivacyError> {
        status.effective_mode = self.collection_policy.effective_mode();
        if status.lifecycle_state == bitfun_product_domains::privacy::PrivacyLifecycleState::Full
            && status.effective_mode != PrivacyEffectiveMode::Full
        {
            status.configuration_error = Some("PRIVACY_POLICY_NOT_APPLIED".to_string());
        }
        Ok(status)
    }

    async fn initialize(&self, app_version: &str) -> Result<(PrivacyStatus, bool), PrivacyError> {
        let Some(service) = self.service.as_ref() else {
            return Ok((PrivacyStatus::disabled(), false));
        };
        let status = service.lock().await.initialize(app_version).await?;
        let mut entered_full_mode = false;
        let application = if status.effective_mode == PrivacyEffectiveMode::Full {
            self.enter_full_mode()
                .map(|entered| entered_full_mode = entered)
        } else {
            self.enter_not_accepted_mode()
        };
        if let Err(error) = application {
            let mut failed_status = status;
            failed_status.effective_mode = PrivacyEffectiveMode::PrivacyNotAccepted;
            failed_status.configuration_error = Some(error.code);
            return Ok((failed_status, false));
        }
        Ok((
            self.status_with_effective_mode(status).await?,
            entered_full_mode,
        ))
    }
}

#[tauri::command]
pub async fn privacy_initialize(
    state: State<'_, PrivacyServiceState>,
    app: tauri::AppHandle,
    _request: InitializePrivacyRequest,
) -> Result<PrivacyStatus, PrivacyError> {
    let (status, entered_full_mode) = state
        .initialize(&app.package_info().version.to_string())
        .await?;
    if entered_full_mode {
        resume_collection_requests(app.clone());
    }
    Ok(status)
}

#[tauri::command]
pub async fn privacy_get_status(
    state: State<'_, PrivacyServiceState>,
    app: tauri::AppHandle,
    request: GetPrivacyStatusRequest,
) -> Result<PrivacyStatus, PrivacyError> {
    if state.service.is_none() {
        return Ok(PrivacyStatus::disabled());
    }
    let app_version = app.package_info().version.to_string();
    let status = state
        .with_service(|service| {
            Box::pin(async move { service.status(&request.locale, &app_version).await })
        })
        .await?;
    state.status_with_effective_mode(status).await
}

#[tauri::command]
pub async fn privacy_accept(
    state: State<'_, PrivacyServiceState>,
    app: tauri::AppHandle,
    request: AcceptPrivacyRequest,
) -> Result<PrivacyStatus, PrivacyError> {
    if state.service.is_none() {
        return Ok(PrivacyStatus::disabled());
    }
    let app_version = app.package_info().version.to_string();
    let status = state
        .with_service(|service| {
            Box::pin(async move { service.accept(&request, &app_version).await })
        })
        .await?;
    if state.enter_full_mode()? {
        resume_collection_requests(app.clone());
    }
    state.status_with_effective_mode(status).await
}

#[tauri::command]
pub async fn privacy_enter_not_accepted(
    state: State<'_, PrivacyServiceState>,
    app: tauri::AppHandle,
    request: EnterPrivacyNotAcceptedRequest,
) -> Result<PrivacyStatus, PrivacyError> {
    if state.service.is_none() {
        return Ok(PrivacyStatus::disabled());
    }
    state.enter_not_accepted_mode()?;
    crate::api::remote_connect_api::suspend_for_privacy().await;
    let app_version = app.package_info().version.to_string();
    let status = state
        .with_service(|service| {
            Box::pin(async move {
                service
                    .enter_not_accepted(&request.locale, &app_version)
                    .await
            })
        })
        .await?;
    state.status_with_effective_mode(status).await
}

#[tauri::command]
pub async fn privacy_mark_viewed(
    state: State<'_, PrivacyServiceState>,
    app: tauri::AppHandle,
    request: MarkPrivacyViewedRequest,
) -> Result<PrivacyStatus, PrivacyError> {
    if state.service.is_none() {
        return Ok(PrivacyStatus::disabled());
    }
    let app_version = app.package_info().version.to_string();
    let status = state
        .with_service(|service| {
            Box::pin(async move {
                service
                    .mark_viewed(&request.policy_version, &app_version, &request.locale)
                    .await
            })
        })
        .await?;
    state.status_with_effective_mode(status).await
}

#[tauri::command]
pub async fn privacy_apply_collection_policy(
    state: State<'_, PrivacyServiceState>,
    app: tauri::AppHandle,
    request: ApplyPrivacyCollectionPolicyRequest,
) -> Result<PrivacyStatus, PrivacyError> {
    if state.service.is_none() {
        return Ok(PrivacyStatus::disabled());
    }
    if request.mode == PrivacyEffectiveMode::Full {
        let full_is_valid = state
            .with_service(|service| Box::pin(async move { service.can_apply_full_mode().await }))
            .await?;
        if !full_is_valid {
            return Err(PrivacyError::new(
                "PRIVACY_CONSENT_REQUIRED",
                "Full collection mode requires a valid saved consent",
            ));
        }
    }
    let entered_full_mode = if request.mode == PrivacyEffectiveMode::Full {
        state.enter_full_mode()?
    } else {
        state.enter_not_accepted_mode()?;
        crate::api::remote_connect_api::suspend_for_privacy().await;
        false
    };
    if entered_full_mode {
        resume_collection_requests(app.clone());
    }
    let app_version = app.package_info().version.to_string();
    let status = state
        .with_service(|service| {
            Box::pin(async move { service.status(&request.locale, &app_version).await })
        })
        .await?;
    state.status_with_effective_mode(status).await
}

fn resume_collection_requests(app: tauri::AppHandle) {
    #[cfg(target_env = "ohos")]
    {
        crate::api::remote_connect_api::init_on_startup();
    }
    #[cfg(not(target_env = "ohos"))]
    let _ = app;
}
