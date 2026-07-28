use bitfun_product_domains::feedback::{
    FeedbackError, SubmitFeedbackRequest, SubmitFeedbackResponse,
};
use bitfun_services_integrations::feedback::FeedbackService;
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
