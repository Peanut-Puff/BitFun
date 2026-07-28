use serde::{Deserialize, Serialize};

pub const FEEDBACK_CONTENT_MAX_CHARS: usize = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackCategory {
    RuntimeError,
    FeatureRequest,
    UsageQuestion,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackStatus {
    Submitted,
    InProgress,
    WaitingUser,
    Resolved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitFeedbackRequest {
    pub category: FeedbackCategory,
    pub content: String,
    #[serde(default)]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub session_id_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitFeedbackResponse {
    pub feedback_id: String,
    pub status: FeedbackStatus,
    pub inbox_cursor: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_seconds: Option<u64>,
}

impl FeedbackError {
    pub fn new(code: &str, message: &str, retryable: bool) -> Self {
        Self {
            code: code.to_string(),
            message: message.to_string(),
            retryable,
            request_id: None,
            retry_after_seconds: None,
        }
    }

    pub fn validation(code: &str, message: &str) -> Self {
        Self::new(code, message, false)
    }
}

pub fn validate_content(content: &str) -> Result<(), FeedbackError> {
    let length = content.trim().chars().count();
    if length == 0 {
        return Err(FeedbackError::validation(
            "CONTENT_EMPTY",
            "Feedback content is required",
        ));
    }
    if length > FEEDBACK_CONTENT_MAX_CHARS {
        return Err(FeedbackError::validation(
            "CONTENT_TOO_LONG",
            "Feedback content exceeds 2000 Unicode characters",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{validate_content, FEEDBACK_CONTENT_MAX_CHARS};

    #[test]
    fn validates_feedback_content_by_unicode_scalar_count() {
        assert_eq!(validate_content(" \n ").unwrap_err().code, "CONTENT_EMPTY");
        assert!(validate_content(&"中".repeat(FEEDBACK_CONTENT_MAX_CHARS)).is_ok());
        assert_eq!(
            validate_content(&"中".repeat(FEEDBACK_CONTENT_MAX_CHARS + 1))
                .unwrap_err()
                .code,
            "CONTENT_TOO_LONG"
        );
    }
}
