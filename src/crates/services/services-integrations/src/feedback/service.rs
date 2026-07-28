use super::vault::{FeedbackCredentialStore, FileFeedbackCredentialStore};
use bitfun_product_domains::feedback::{
    validate_content, validate_inbox_page_size, FeedbackAccessState, FeedbackError,
    FeedbackInboxPage, FeedbackRecordSummary, FeedbackStatus, ListFeedbackRecordsRequest,
    SubmitFeedbackRequest, SubmitFeedbackResponse,
};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use reqwest::{Response, StatusCode};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use uuid::Uuid;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const ACCESS_TOKEN_REFRESH_MARGIN_SECONDS: i64 = 600;
const DEBUG_FEEDBACK_API_BASE_URL: &str = "http://127.0.0.1:38971";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct StoredCredentials {
    enroll_key: String,
    #[serde(default)]
    enroll_idempotency_key: Option<String>,
    #[serde(default)]
    refresh_idempotency_key: Option<String>,
    #[serde(default)]
    anonymous_id: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    capabilities: HashMap<String, String>,
    #[serde(default)]
    pending_create_fingerprint: Option<String>,
    #[serde(default)]
    pending_create_idempotency_key: Option<String>,
    #[serde(default)]
    inbox_items: Vec<FeedbackRecordSummary>,
    #[serde(default)]
    inbox_next_cursor: Option<String>,
    #[serde(default)]
    inbox_has_more: bool,
}

#[derive(Debug, Clone)]
struct AccessToken {
    value: String,
    expires_at: DateTime<Utc>,
    scopes: Vec<String>,
}

#[derive(Debug, Default)]
struct RuntimeState {
    loaded: bool,
    stored: StoredCredentials,
    access_token: Option<AccessToken>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    anonymous_id: String,
    access_token: String,
    refresh_token: String,
    expires_in: i64,
    scope: String,
}

#[derive(Debug, Deserialize)]
struct CreateResponse {
    feedback_id: String,
    capability_token: String,
    status: FeedbackStatus,
    inbox_cursor: String,
}

#[derive(Debug, Serialize)]
struct CreateRequestBody<'a> {
    category: bitfun_product_domains::feedback::FeedbackCategory,
    content: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    trace_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id_hash: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_version: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct InboxResponse {
    items: Vec<InboxItem>,
    cursor: String,
    has_more: bool,
}

#[derive(Debug, Deserialize)]
struct InboxItem {
    feedback_id: String,
    category: bitfun_product_domains::feedback::FeedbackCategory,
    status: FeedbackStatus,
    has_new_reply: bool,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Default, Deserialize)]
struct ServerErrorBody {
    error_code: Option<String>,
    request_id: Option<String>,
}

pub struct FeedbackService {
    client: reqwest::Client,
    base_url: Option<String>,
    client_version: String,
    credential_store: Arc<dyn FeedbackCredentialStore>,
    state: Mutex<RuntimeState>,
}

impl FeedbackService {
    pub fn from_environment(data_dir: PathBuf, client_version: impl Into<String>) -> Self {
        Self::from_environment_with_credential_store(
            client_version,
            Arc::new(FileFeedbackCredentialStore::new(data_dir)),
        )
    }

    pub fn from_environment_with_credential_store(
        client_version: impl Into<String>,
        credential_store: Arc<dyn FeedbackCredentialStore>,
    ) -> Self {
        Self::new(
            configured_base_url(),
            client_version.into(),
            credential_store,
            REQUEST_TIMEOUT,
        )
    }

    fn new(
        base_url: Option<String>,
        client_version: String,
        credential_store: Arc<dyn FeedbackCredentialStore>,
        timeout: Duration,
    ) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(timeout)
                .build()
                .expect("feedback HTTP client must initialize"),
            base_url,
            client_version,
            credential_store,
            state: Mutex::new(RuntimeState::default()),
        }
    }

    pub async fn submit_feedback(
        &self,
        request: SubmitFeedbackRequest,
    ) -> Result<SubmitFeedbackResponse, FeedbackError> {
        let request = normalize_request(request);
        validate_content(&request.content)?;
        let idempotency_key = self.create_idempotency_key(&request).await?;
        let body = CreateRequestBody {
            category: request.category,
            content: request.content.trim(),
            trace_id: request.trace_id.as_deref(),
            session_id_hash: request.session_id_hash.as_deref(),
            client_version: valid_optional_value(&self.client_version, 20),
        };
        let response = self
            .send_authenticated("feedback:write", |token| {
                self.client
                    .post(self.url("/support/v1/feedback")?)
                    .bearer_auth(token)
                    .header("X-Request-ID", Uuid::new_v4().to_string())
                    .header("Idempotency-Key", &idempotency_key)
                    .json(&body)
                    .build()
                    .map_err(network_error)
            })
            .await?;
        let created: CreateResponse = decode_success(response, StatusCode::CREATED).await?;
        if created.status != FeedbackStatus::Submitted {
            return Err(FeedbackError::new(
                "RESPONSE_INVALID",
                "Feedback service returned an invalid initial status",
                true,
            ));
        }

        let mut state = self.state.lock().await;
        self.ensure_loaded(&mut state).await?;
        let mut next = state.stored.clone();
        next.capabilities
            .insert(created.feedback_id.clone(), created.capability_token);
        next.pending_create_fingerprint = None;
        next.pending_create_idempotency_key = None;
        if let Err(error) = self.persist(&next).await {
            return Err(FeedbackError {
                code: "CAPABILITY_SAVE_FAILED".to_string(),
                message: "Feedback access could not be saved securely".to_string(),
                retryable: true,
                request_id: error.request_id,
                retry_after_seconds: None,
            });
        }
        state.stored = next;
        Ok(SubmitFeedbackResponse {
            feedback_id: created.feedback_id,
            status: created.status,
            inbox_cursor: created.inbox_cursor,
        })
    }

    pub async fn access_state(&self) -> Result<FeedbackAccessState, FeedbackError> {
        let mut state = self.state.lock().await;
        self.ensure_existing_loaded(&mut state).await?;
        let can_reuse_access =
            state.stored.anonymous_id.is_some() && state.stored.refresh_token.is_some();
        let has_history =
            !state.stored.capabilities.is_empty() || !state.stored.inbox_items.is_empty();
        Ok(FeedbackAccessState {
            has_history,
            can_reuse_access,
            cached_inbox: cached_inbox(&state.stored, can_reuse_access),
        })
    }

    pub async fn list_feedback(
        &self,
        request: ListFeedbackRecordsRequest,
    ) -> Result<FeedbackInboxPage, FeedbackError> {
        validate_inbox_page_size(request.page_size)?;
        let cursor = request.cursor.clone();
        let page_size = request.page_size.to_string();
        let response = self
            .send_authenticated_existing("feedback:read", |token| {
                let mut request = self
                    .client
                    .get(self.url("/support/v1/feedback/inbox")?)
                    .bearer_auth(token)
                    .header("X-Request-ID", Uuid::new_v4().to_string())
                    .query(&[("limit", &page_size)]);
                if let Some(cursor) = cursor.as_deref() {
                    request = request.query(&[("cursor", cursor)]);
                }
                request.build().map_err(network_error)
            })
            .await?;
        let received: InboxResponse = decode_success(response, StatusCode::OK).await?;

        let mut state = self.state.lock().await;
        self.ensure_existing_loaded(&mut state).await?;
        let can_reuse_access =
            state.stored.anonymous_id.is_some() && state.stored.refresh_token.is_some();
        let items = received
            .items
            .into_iter()
            .map(|item| FeedbackRecordSummary {
                can_open: can_reuse_access
                    && state.stored.capabilities.contains_key(&item.feedback_id),
                feedback_id: item.feedback_id,
                category: item.category,
                status: item.status,
                has_new_reply: item.has_new_reply,
                created_at: item.created_at,
                updated_at: item.updated_at,
            })
            .collect::<Vec<_>>();
        let next_cursor = (!received.cursor.is_empty()).then_some(received.cursor);

        let mut next = state.stored.clone();
        if request.cursor.is_none() {
            next.inbox_items = items.clone();
        } else {
            for item in &items {
                if let Some(existing) = next
                    .inbox_items
                    .iter_mut()
                    .find(|existing| existing.feedback_id == item.feedback_id)
                {
                    *existing = item.clone();
                } else {
                    next.inbox_items.push(item.clone());
                }
            }
        }
        next.inbox_next_cursor = next_cursor.clone();
        next.inbox_has_more = received.has_more;
        self.persist(&next).await?;
        state.stored = next;

        Ok(FeedbackInboxPage {
            items,
            next_cursor,
            has_more: received.has_more,
        })
    }

    fn url(&self, path: &str) -> Result<String, FeedbackError> {
        self.base_url
            .as_ref()
            .map(|base| format!("{base}{path}"))
            .ok_or_else(|| {
                FeedbackError::new(
                    "FEEDBACK_NOT_CONFIGURED",
                    "Feedback API base URL is not configured",
                    false,
                )
            })
    }

    async fn create_idempotency_key(
        &self,
        request: &SubmitFeedbackRequest,
    ) -> Result<String, FeedbackError> {
        let fingerprint = request_fingerprint(request)?;
        let mut state = self.state.lock().await;
        self.ensure_loaded(&mut state).await?;
        if state.stored.pending_create_fingerprint.as_deref() == Some(&fingerprint) {
            if let Some(key) = state.stored.pending_create_idempotency_key.as_ref() {
                return Ok(key.clone());
            }
        }

        let key = Uuid::new_v4().to_string();
        let mut next = state.stored.clone();
        next.pending_create_fingerprint = Some(fingerprint);
        next.pending_create_idempotency_key = Some(key.clone());
        self.persist(&next).await?;
        state.stored = next;
        Ok(key)
    }

    async fn send_authenticated<F>(
        &self,
        scope: &str,
        build_request: F,
    ) -> Result<Response, FeedbackError>
    where
        F: Fn(&str) -> Result<reqwest::Request, FeedbackError>,
    {
        let token = self.access_token(scope, false).await?;
        let response = self
            .client
            .execute(build_request(&token)?)
            .await
            .map_err(network_error)?;
        if response.status() != StatusCode::UNAUTHORIZED {
            return Ok(response);
        }

        let token = self.access_token(scope, true).await?;
        self.client
            .execute(build_request(&token)?)
            .await
            .map_err(network_error)
    }

    async fn send_authenticated_existing<F>(
        &self,
        scope: &str,
        build_request: F,
    ) -> Result<Response, FeedbackError>
    where
        F: Fn(&str) -> Result<reqwest::Request, FeedbackError>,
    {
        let token = self.existing_access_token(scope, false).await?;
        let response = self
            .client
            .execute(build_request(&token)?)
            .await
            .map_err(network_error)?;
        if response.status() != StatusCode::UNAUTHORIZED {
            return Ok(response);
        }

        let token = self.existing_access_token(scope, true).await?;
        self.client
            .execute(build_request(&token)?)
            .await
            .map_err(network_error)
    }

    async fn existing_access_token(
        &self,
        scope: &str,
        force_refresh: bool,
    ) -> Result<String, FeedbackError> {
        let mut state = self.state.lock().await;
        self.ensure_existing_loaded(&mut state).await?;
        if !force_refresh {
            if let Some(token) = state.access_token.as_ref() {
                if token.expires_at
                    > Utc::now() + ChronoDuration::seconds(ACCESS_TOKEN_REFRESH_MARGIN_SECONDS)
                    && token.scopes.iter().any(|item| item == scope)
                {
                    return Ok(token.value.clone());
                }
            }
        }
        if state.stored.refresh_token.is_none() || state.stored.anonymous_id.is_none() {
            return Err(FeedbackError::new(
                "FEEDBACK_ACCESS_UNAVAILABLE",
                "Saved feedback access is unavailable",
                false,
            ));
        }
        let token = match self.refresh(&mut state).await {
            Ok(token) => token,
            Err(error) if refresh_requires_enroll(&error.code) => {
                let mut next = state.stored.clone();
                next.anonymous_id = None;
                next.refresh_token = None;
                next.refresh_idempotency_key = None;
                self.persist(&next).await?;
                state.stored = next;
                state.access_token = None;
                return Err(FeedbackError::new(
                    "FEEDBACK_ACCESS_EXPIRED",
                    "Saved feedback access has expired",
                    false,
                ));
            }
            Err(error) => return Err(error),
        };
        if !token.scopes.iter().any(|item| item == scope) {
            return Err(FeedbackError::new(
                "SCOPE_INSUFFICIENT",
                "The feedback token does not include the required scope",
                false,
            ));
        }
        let value = token.value.clone();
        state.access_token = Some(token);
        Ok(value)
    }

    async fn access_token(
        &self,
        scope: &str,
        force_refresh: bool,
    ) -> Result<String, FeedbackError> {
        let mut state = self.state.lock().await;
        self.ensure_loaded(&mut state).await?;
        if !force_refresh {
            if let Some(token) = state.access_token.as_ref() {
                if token.expires_at
                    > Utc::now() + ChronoDuration::seconds(ACCESS_TOKEN_REFRESH_MARGIN_SECONDS)
                    && token.scopes.iter().any(|item| item == scope)
                {
                    return Ok(token.value.clone());
                }
            }
        }

        let token = if state.stored.refresh_token.is_some() {
            match self.refresh(&mut state).await {
                Ok(token) => token,
                Err(error) if refresh_requires_enroll(&error.code) => {
                    let mut next = state.stored.clone();
                    next.anonymous_id = None;
                    next.refresh_token = None;
                    next.refresh_idempotency_key = None;
                    next.capabilities.clear();
                    self.persist(&next).await?;
                    state.stored = next;
                    state.access_token = None;
                    self.enroll(&mut state).await?
                }
                Err(error) => return Err(error),
            }
        } else {
            self.enroll(&mut state).await?
        };
        if !token.scopes.iter().any(|item| item == scope) {
            return Err(FeedbackError::new(
                "SCOPE_INSUFFICIENT",
                "The feedback token does not include the required scope",
                false,
            ));
        }
        let value = token.value.clone();
        state.access_token = Some(token);
        Ok(value)
    }

    async fn enroll(&self, state: &mut RuntimeState) -> Result<AccessToken, FeedbackError> {
        let idempotency_key = match state.stored.enroll_idempotency_key.as_ref() {
            Some(key) => key.clone(),
            None => {
                let mut next = state.stored.clone();
                let key = Uuid::new_v4().to_string();
                next.enroll_idempotency_key = Some(key.clone());
                self.persist(&next).await?;
                state.stored = next;
                key
            }
        };
        let response = self
            .client
            .post(self.url("/auth/v1/anonymous/enroll")?)
            .header("X-Request-ID", Uuid::new_v4().to_string())
            .header("Idempotency-Key", idempotency_key)
            .json(&serde_json::json!({ "key": state.stored.enroll_key }))
            .send()
            .await
            .map_err(network_error)?;
        let token: TokenResponse = decode_success(response, StatusCode::CREATED).await?;
        self.commit_token(state, token, true).await
    }

    async fn refresh(&self, state: &mut RuntimeState) -> Result<AccessToken, FeedbackError> {
        let refresh_token = state.stored.refresh_token.clone().ok_or_else(|| {
            FeedbackError::new(
                "REFRESH_TOKEN_MISSING",
                "Feedback refresh token is unavailable",
                false,
            )
        })?;
        let idempotency_key = match state.stored.refresh_idempotency_key.as_ref() {
            Some(key) => key.clone(),
            None => {
                let mut next = state.stored.clone();
                let key = Uuid::new_v4().to_string();
                next.refresh_idempotency_key = Some(key.clone());
                self.persist(&next).await?;
                state.stored = next;
                key
            }
        };
        let response = self
            .client
            .post(self.url("/auth/v1/anonymous/token")?)
            .header("X-Request-ID", Uuid::new_v4().to_string())
            .header("Idempotency-Key", idempotency_key)
            .json(&serde_json::json!({ "refresh_token": refresh_token }))
            .send()
            .await
            .map_err(network_error)?;
        let token: TokenResponse = decode_success(response, StatusCode::OK).await?;
        self.commit_token(state, token, false).await
    }

    async fn commit_token(
        &self,
        state: &mut RuntimeState,
        response: TokenResponse,
        enrolled: bool,
    ) -> Result<AccessToken, FeedbackError> {
        let token = AccessToken {
            value: response.access_token,
            expires_at: Utc::now() + ChronoDuration::seconds(response.expires_in.max(0)),
            scopes: parse_scopes(&response.scope),
        };
        let mut next = state.stored.clone();
        next.anonymous_id = Some(response.anonymous_id);
        next.refresh_token = Some(response.refresh_token);
        next.refresh_idempotency_key = None;
        if enrolled {
            next.enroll_idempotency_key = None;
        }
        self.persist(&next).await?;
        state.stored = next;
        Ok(token)
    }

    async fn ensure_loaded(&self, state: &mut RuntimeState) -> Result<(), FeedbackError> {
        self.ensure_existing_loaded(state).await?;
        if state.stored.enroll_key.is_empty() {
            state.stored.enroll_key = Uuid::new_v4().to_string();
            self.persist(&state.stored).await?;
        }
        Ok(())
    }

    async fn ensure_existing_loaded(&self, state: &mut RuntimeState) -> Result<(), FeedbackError> {
        if state.loaded {
            return Ok(());
        }
        let stored = self.credential_store.load().await.map_err(|_| {
            credential_error(
                "CREDENTIAL_LOAD_FAILED",
                "Feedback access could not be loaded",
            )
        })?;
        state.stored = match stored {
            Some(value) => serde_json::from_str(&value).map_err(|_| {
                credential_error(
                    "CREDENTIALS_INVALID",
                    "Saved feedback access data is invalid",
                )
            })?,
            None => StoredCredentials::default(),
        };
        state.loaded = true;
        Ok(())
    }

    async fn persist(&self, stored: &StoredCredentials) -> Result<(), FeedbackError> {
        let value = serde_json::to_string(stored).map_err(|_| {
            credential_error(
                "CREDENTIAL_SAVE_FAILED",
                "Feedback access could not be encoded",
            )
        })?;
        self.credential_store.store(&value).await.map_err(|_| {
            credential_error(
                "CREDENTIAL_SAVE_FAILED",
                "Feedback access could not be saved securely",
            )
        })
    }
}

fn cached_inbox(stored: &StoredCredentials, can_reuse_access: bool) -> FeedbackInboxPage {
    FeedbackInboxPage {
        items: stored
            .inbox_items
            .iter()
            .cloned()
            .map(|mut item| {
                item.can_open =
                    can_reuse_access && stored.capabilities.contains_key(&item.feedback_id);
                item
            })
            .collect(),
        next_cursor: stored.inbox_next_cursor.clone(),
        has_more: stored.inbox_has_more,
    }
}

fn configured_base_url() -> Option<String> {
    let configured = std::env::var("BITFUN_FEEDBACK_API_BASE_URL")
        .ok()
        .or_else(|| option_env!("BITFUN_FEEDBACK_API_BASE_URL").map(ToString::to_string))
        .map(|value| value.trim_end_matches('/').to_string())
        .filter(|value| {
            value.starts_with("https://")
                || (cfg!(debug_assertions) && value.starts_with("http://"))
        });
    if cfg!(debug_assertions) {
        configured.or_else(|| Some(DEBUG_FEEDBACK_API_BASE_URL.to_string()))
    } else {
        configured
    }
}

fn request_fingerprint(request: &SubmitFeedbackRequest) -> Result<String, FeedbackError> {
    let encoded = serde_json::to_vec(request).map_err(|_| {
        FeedbackError::new(
            "REQUEST_ENCODING_FAILED",
            "Feedback request could not be encoded",
            false,
        )
    })?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}

fn normalize_request(mut request: SubmitFeedbackRequest) -> SubmitFeedbackRequest {
    request.trace_id = valid_optional_value_owned(request.trace_id, 64);
    request.session_id_hash = valid_optional_value_owned(request.session_id_hash, 64);
    request
}

fn valid_optional_value(value: &str, max_chars: usize) -> Option<&str> {
    (!value.is_empty() && value.chars().count() <= max_chars).then_some(value)
}

fn valid_optional_value_owned(value: Option<String>, max_chars: usize) -> Option<String> {
    value.filter(|item| !item.is_empty() && item.chars().count() <= max_chars)
}

fn parse_scopes(value: &str) -> Vec<String> {
    value
        .split(|character: char| character == ',' || character.is_whitespace())
        .map(str::trim)
        .filter(|scope| !scope.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn refresh_requires_enroll(code: &str) -> bool {
    matches!(
        code,
        "REFRESH_TOKEN_INVALID" | "REFRESH_TOKEN_REUSED" | "TOKEN_FAMILY_REVOKED"
    )
}

async fn decode_success<T: DeserializeOwned>(
    response: Response,
    expected: StatusCode,
) -> Result<T, FeedbackError> {
    if response.status() != expected {
        return Err(decode_error(response).await);
    }
    response.json::<T>().await.map_err(|_| {
        FeedbackError::new(
            "RESPONSE_INVALID",
            "Feedback service returned an invalid response",
            true,
        )
    })
}

async fn decode_error(response: Response) -> FeedbackError {
    let status = response.status();
    let request_id_header = response
        .headers()
        .get("X-Request-ID")
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);
    let retry_after_seconds = response
        .headers()
        .get("Retry-After")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    let body = response.json::<ServerErrorBody>().await.unwrap_or_default();
    let code = body.error_code.unwrap_or_else(|| match status {
        StatusCode::UNAUTHORIZED => "ACCESS_TOKEN_INVALID".to_string(),
        StatusCode::FORBIDDEN => "ACCESS_FORBIDDEN".to_string(),
        StatusCode::TOO_MANY_REQUESTS => "RATE_LIMITED".to_string(),
        status if status.is_server_error() => "SERVICE_UNAVAILABLE".to_string(),
        _ => "REQUEST_REJECTED".to_string(),
    });
    let retryable = status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
    FeedbackError {
        message: safe_error_message(&code).to_string(),
        code,
        retryable,
        request_id: body.request_id.or(request_id_header),
        retry_after_seconds,
    }
}

fn safe_error_message(code: &str) -> &'static str {
    match code {
        "CONTENT_EMPTY" | "CONTENT_TOO_LONG" | "CONTENT_UNSAFE" | "CATEGORY_INVALID" => {
            "Feedback content was rejected"
        }
        "SCOPE_INSUFFICIENT" | "INSTANCE_BANNED" | "ACCESS_FORBIDDEN" => {
            "Feedback access is not permitted"
        }
        "RATE_LIMITED" | "FEEDBACK_QUOTA_EXCEEDED" | "QUOTA_EXCEEDED" => {
            "Feedback requests are temporarily limited"
        }
        _ => "Feedback request could not be completed",
    }
}

fn network_error(error: reqwest::Error) -> FeedbackError {
    if error.is_timeout() {
        FeedbackError::new("REQUEST_TIMEOUT", "Feedback request timed out", true)
    } else {
        FeedbackError::new("NETWORK_ERROR", "Feedback service is unavailable", true)
    }
}

fn credential_error(code: &str, message: &str) -> FeedbackError {
    FeedbackError::new(code, message, true)
}

#[cfg(test)]
mod tests {
    use super::{normalize_request, parse_scopes, FeedbackService, StoredCredentials};
    use crate::feedback::FeedbackCredentialStore;
    use anyhow::{anyhow, Result};
    use async_trait::async_trait;
    use bitfun_product_domains::feedback::{
        FeedbackCategory, ListFeedbackRecordsRequest, SubmitFeedbackRequest,
    };
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex as StdMutex};
    use std::time::Duration;

    #[derive(Default)]
    struct MemoryStore {
        value: StdMutex<Option<String>>,
        stores: AtomicUsize,
        fail_at: AtomicUsize,
    }

    #[async_trait]
    impl FeedbackCredentialStore for MemoryStore {
        async fn load(&self) -> Result<Option<String>> {
            Ok(self.value.lock().unwrap().clone())
        }

        async fn store(&self, value: &str) -> Result<()> {
            let call = self.stores.fetch_add(1, Ordering::SeqCst) + 1;
            if self.fail_at.load(Ordering::SeqCst) == call {
                return Err(anyhow!("injected store failure"));
            }
            *self.value.lock().unwrap() = Some(value.to_string());
            Ok(())
        }
    }

    #[test]
    fn accepts_backend_scope_delimiters() {
        assert_eq!(
            parse_scopes("config:read,feedback:write feedback:read"),
            vec!["config:read", "feedback:write", "feedback:read"]
        );
    }

    #[test]
    fn omits_invalid_optional_correlation_values() {
        let normalized = normalize_request(SubmitFeedbackRequest {
            category: FeedbackCategory::Other,
            content: "feedback".to_string(),
            trace_id: Some(String::new()),
            session_id_hash: Some("x".repeat(65)),
        });
        assert_eq!(normalized.trace_id, None);
        assert_eq!(normalized.session_id_hash, None);
    }

    #[tokio::test]
    async fn access_state_does_not_create_an_identity() {
        let store = Arc::new(MemoryStore::default());
        let service = FeedbackService::new(
            None,
            "1.0.0".to_string(),
            store.clone(),
            Duration::from_secs(2),
        );

        let state = service.access_state().await.unwrap();

        assert!(!state.has_history);
        assert!(!state.can_reuse_access);
        assert_eq!(store.stores.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn lists_inbox_with_backend_cursor_mapping_and_cached_accessibility() {
        let responses = vec![
            json_response(
                200,
                r#"{"anonymous_id":"anon","access_token":"fresh","refresh_token":"refresh-2","expires_in":3600,"refresh_expires_in":2592000,"scope":"feedback:write,feedback:read","schema_version":"1"}"#,
            ),
            json_response(
                200,
                r#"{"items":[{"feedback_id":"feedback-1","category":"other","status":"waiting_user","has_new_reply":true,"created_at":"2026-07-28T01:00:00Z","updated_at":"2026-07-28T02:00:00Z"}],"cursor":"cursor-2","has_more":true}"#,
            ),
        ];
        let (base_url, requests) = spawn_server(responses).await;
        let stored = StoredCredentials {
            enroll_key: "enroll".to_string(),
            anonymous_id: Some("anon".to_string()),
            refresh_token: Some("refresh-1".to_string()),
            capabilities: HashMap::from([("feedback-1".to_string(), "capability".to_string())]),
            ..StoredCredentials::default()
        };
        let store = Arc::new(MemoryStore {
            value: StdMutex::new(Some(serde_json::to_string(&stored).unwrap())),
            ..MemoryStore::default()
        });
        let service = FeedbackService::new(
            Some(base_url),
            "1.0.0".to_string(),
            store,
            Duration::from_secs(2),
        );

        let page = service
            .list_feedback(ListFeedbackRecordsRequest {
                cursor: Some("cursor-1".to_string()),
                page_size: 20,
            })
            .await
            .unwrap();

        assert_eq!(page.next_cursor.as_deref(), Some("cursor-2"));
        assert!(page.has_more);
        assert!(page.items[0].can_open);
        let captured = requests.lock().unwrap();
        assert!(captured[1].starts_with("GET /support/v1/feedback/inbox?limit=20&cursor=cursor-1 "));
        drop(captured);
        let restored = service.access_state().await.unwrap();
        assert!(restored.cached_inbox.items[0].has_new_reply);
    }

    #[tokio::test]
    async fn capability_persistence_is_part_of_submission_success() {
        let responses = vec![
            json_response(
                201,
                r#"{"anonymous_id":"anon","access_token":"access","refresh_token":"refresh","expires_in":3600,"refresh_expires_in":2592000,"scope":"feedback:write,feedback:read","schema_version":"1"}"#,
            ),
            json_response(
                201,
                r#"{"feedback_id":"feedback-1","capability_token":"capability","status":"submitted","inbox_cursor":"cursor-1","schema_version":"1"}"#,
            ),
            json_response(
                201,
                r#"{"feedback_id":"feedback-1","capability_token":"capability","status":"submitted","inbox_cursor":"cursor-1","schema_version":"1","idempotency_replayed":true}"#,
            ),
        ];
        let (base_url, requests) = spawn_server(responses).await;
        let store = Arc::new(MemoryStore::default());
        store.fail_at.store(5, Ordering::SeqCst);
        let service = FeedbackService::new(
            Some(base_url),
            "1.0.0".to_string(),
            store,
            Duration::from_secs(2),
        );
        let request = SubmitFeedbackRequest {
            category: FeedbackCategory::Other,
            content: "privacy-safe feedback".to_string(),
            trace_id: None,
            session_id_hash: None,
        };

        let first = service.submit_feedback(request.clone()).await.unwrap_err();
        assert_eq!(first.code, "CAPABILITY_SAVE_FAILED");
        let second = service.submit_feedback(request).await.unwrap();
        assert_eq!(second.feedback_id, "feedback-1");

        let captured = requests.lock().unwrap();
        let create_keys: Vec<_> = captured
            .iter()
            .filter(|request| request.starts_with("POST /support/v1/feedback "))
            .filter_map(|request| header(request, "idempotency-key"))
            .collect();
        assert_eq!(create_keys.len(), 2);
        assert_eq!(create_keys[0], create_keys[1]);
    }

    #[tokio::test]
    async fn recovers_from_one_unauthorized_response_and_replays_once() {
        let responses = vec![
            json_response(
                201,
                r#"{"anonymous_id":"anon","access_token":"expired","refresh_token":"refresh-1","expires_in":3600,"refresh_expires_in":2592000,"scope":"feedback:write,feedback:read","schema_version":"1"}"#,
            ),
            json_response(
                401,
                r#"{"error_code":"ACCESS_TOKEN_INVALID","error_message":"expired","request_id":"request-401"}"#,
            ),
            json_response(
                200,
                r#"{"anonymous_id":"anon","access_token":"fresh","refresh_token":"refresh-2","expires_in":3600,"refresh_expires_in":2592000,"scope":"feedback:write,feedback:read","schema_version":"1"}"#,
            ),
            json_response(
                201,
                r#"{"feedback_id":"feedback-1","capability_token":"capability","status":"submitted","inbox_cursor":"cursor-1","schema_version":"1"}"#,
            ),
        ];
        let (base_url, requests) = spawn_server(responses).await;
        let service = FeedbackService::new(
            Some(base_url),
            "1.0.0".to_string(),
            Arc::new(MemoryStore::default()),
            Duration::from_secs(2),
        );
        service
            .submit_feedback(SubmitFeedbackRequest {
                category: FeedbackCategory::Other,
                content: "recover once".to_string(),
                trace_id: None,
                session_id_hash: None,
            })
            .await
            .unwrap();

        let captured = requests.lock().unwrap();
        assert!(captured[0].starts_with("POST /auth/v1/anonymous/enroll "));
        assert!(captured[1].starts_with("POST /support/v1/feedback "));
        assert!(captured[2].starts_with("POST /auth/v1/anonymous/token "));
        assert!(captured[3].starts_with("POST /support/v1/feedback "));
        assert_eq!(
            header(&captured[1], "idempotency-key"),
            header(&captured[3], "idempotency-key")
        );
    }

    fn json_response(status: u16, body: &str) -> String {
        format!(
            "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    async fn spawn_server(responses: Vec<String>) -> (String, Arc<StdMutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let captured = requests.clone();
        tokio::spawn(async move {
            for response in responses {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0u8; 4096];
                loop {
                    let read = stream.read(&mut buffer).await.unwrap();
                    if read == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buffer[..read]);
                    if request_is_complete(&bytes) {
                        break;
                    }
                }
                captured
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&bytes).into_owned());
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        (format!("http://{address}"), requests)
    }

    fn request_is_complete(bytes: &[u8]) -> bool {
        let request = String::from_utf8_lossy(bytes);
        let Some(header_end) = request.find("\r\n\r\n") else {
            return false;
        };
        let content_length = request[..header_end]
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
            .unwrap_or(0);
        bytes.len() >= header_end + 4 + content_length
    }

    fn header(request: &str, name: &str) -> Option<String> {
        request.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name)
                .then(|| value.trim().to_string())
        })
    }
}
