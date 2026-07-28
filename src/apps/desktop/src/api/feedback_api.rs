use bitfun_product_domains::feedback::{
    FeedbackAccessState, FeedbackError, FeedbackInboxPage, ListFeedbackRecordsRequest,
    SubmitFeedbackRequest, SubmitFeedbackResponse,
};
use bitfun_services_integrations::feedback::FeedbackService;
use serde::Deserialize;
use tauri::State;

use crate::api::privacy_api::PrivacyServiceState;

pub struct FeedbackServiceState {
    service: Option<FeedbackService>,
}

impl FeedbackServiceState {
    pub fn enabled(service: FeedbackService) -> Self {
        Self {
            service: Some(service),
        }
    }

    pub fn disabled() -> Self {
        Self { service: None }
    }

    fn service(&self) -> Result<&FeedbackService, FeedbackError> {
        self.service.as_ref().ok_or_else(|| {
            FeedbackError::new(
                "FEEDBACK_PLATFORM_UNSUPPORTED",
                "In-app feedback is only available on OpenHarmony",
                false,
            )
        })
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackAccessStateRequest {}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListFeedbackCommandRequest {
    #[serde(default)]
    pub cursor: Option<String>,
    pub page_size: u16,
    #[serde(default)]
    pub user_initiated: bool,
}

#[tauri::command]
pub async fn feedback_get_access_state(
    feedback_state: State<'_, FeedbackServiceState>,
    request: FeedbackAccessStateRequest,
) -> Result<FeedbackAccessState, FeedbackError> {
    let _ = request;
    feedback_state.service()?.access_state().await
}

#[tauri::command]
pub async fn list_feedback(
    feedback_state: State<'_, FeedbackServiceState>,
    privacy_state: State<'_, PrivacyServiceState>,
    request: ListFeedbackCommandRequest,
) -> Result<FeedbackInboxPage, FeedbackError> {
    if !privacy_state.collection_allowed() && !request.user_initiated {
        return Err(FeedbackError::new(
            "PRIVACY_BACKGROUND_REQUEST_BLOCKED",
            "Background feedback requests require full privacy mode",
            false,
        ));
    }
    feedback_state
        .service()?
        .list_feedback(ListFeedbackRecordsRequest {
            cursor: request.cursor,
            page_size: request.page_size,
        })
        .await
}

#[tauri::command]
pub async fn submit_feedback(
    feedback_state: State<'_, FeedbackServiceState>,
    privacy_state: State<'_, PrivacyServiceState>,
    request: SubmitFeedbackRequest,
) -> Result<SubmitFeedbackResponse, FeedbackError> {
    if !privacy_state.collection_allowed() {
        return Err(FeedbackError::new(
            "PRIVACY_CONSENT_REQUIRED",
            "Feedback submission requires full privacy mode",
            false,
        ));
    }
    feedback_state.service()?.submit_feedback(request).await
}
