//! The service's one error shape: `{"error": {"code", "message"}}` with an
//! HTTP status derived from the code.
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// Stable error codes added by the service. Engine codes (`E_UNAVAILABLE`,
/// `E_FORBIDDEN`, `E_BUDGET`, ...) pass through unchanged.
pub mod codes {
    /// Every authentication failure: no credential, a malformed, unknown,
    /// expired or revoked one, or a principal that is not an active user.
    pub const UNAUTHENTICATED: &str = "E_UNAUTHENTICATED";
    /// The request body is larger than the configured limit.
    pub const BODY_LIMIT: &str = "E_BODY_LIMIT";
    /// The request is malformed (JSON, fields, path or query parameters).
    pub const INPUT: &str = "E_INPUT";
    /// The program uses a command the service does not accept.
    pub const UNSUPPORTED: &str = "E_UNSUPPORTED";
    /// A non-administrator asserted a record ID that `POST /v1/ids` did not
    /// mint for it.
    pub const ID_NOT_MINTED: &str = "E_ID_NOT_MINTED";
    /// The tenant's request queue is full.
    pub const BACKPRESSURE: &str = "E_BACKPRESSURE";
    /// The audit entry could not be recorded; the response is withheld.
    pub const AUDIT: &str = "E_AUDIT";
    /// No such route (only after authentication).
    pub const NOT_FOUND: &str = "E_NOT_FOUND";
    /// The route exists but not with this method (only after authentication).
    pub const METHOD: &str = "E_METHOD";
    /// An internal failure. The message never carries storage details.
    pub const INTERNAL: &str = "E_INTERNAL";
}

/// The exact message of every authentication failure.
pub const UNAUTHENTICATED_MESSAGE: &str = "authentication required";

/// The error code of a response, kept in its extensions for the request log.
#[derive(Debug, Clone)]
pub struct ErrorCode(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: String,
    pub message: String,
}

impl ApiError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            status: status_of(code),
            code: code.into(),
            message: message.into(),
        }
    }
    pub fn unauthenticated() -> Self {
        Self::new(codes::UNAUTHENTICATED, UNAUTHENTICATED_MESSAGE)
    }
    pub fn input(message: impl Into<String>) -> Self {
        Self::new(codes::INPUT, message)
    }
    pub fn internal() -> Self {
        Self::new(codes::INTERNAL, "internal error")
    }
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new("E_FORBIDDEN", message)
    }
    /// The JSON body of this error.
    pub fn body(&self) -> Vec<u8> {
        serde_json::to_vec(&json!({"error": {"code": self.code, "message": self.message}}))
            .expect("error bodies serialize")
    }
}

impl From<weave_engine::Error> for ApiError {
    fn from(error: weave_engine::Error) -> Self {
        let status = status_of(&error.code);
        if status.is_server_error() {
            // Storage and integrity messages can carry SQL or paths.
            return Self {
                status,
                code: error.code,
                message: "internal error".into(),
            };
        }
        Self {
            status,
            code: error.code,
            message: error.message,
        }
    }
}

/// The HTTP status for an error code.
pub fn status_of(code: &str) -> StatusCode {
    match code {
        codes::UNAUTHENTICATED => StatusCode::UNAUTHORIZED,
        "E_FORBIDDEN" => StatusCode::FORBIDDEN,
        "E_UNAVAILABLE" | codes::NOT_FOUND => StatusCode::NOT_FOUND,
        codes::METHOD => StatusCode::METHOD_NOT_ALLOWED,
        "E_CONFLICT" | "E_HEAD_CONFLICT" => StatusCode::CONFLICT,
        codes::BODY_LIMIT => StatusCode::PAYLOAD_TOO_LARGE,
        "E_BUDGET" => StatusCode::UNPROCESSABLE_ENTITY,
        codes::BACKPRESSURE | codes::AUDIT => StatusCode::SERVICE_UNAVAILABLE,
        codes::INTERNAL | "E_STORAGE" | "E_INTEGRITY" | "E_CLOCK_UNAVAILABLE" => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
        _ => StatusCode::BAD_REQUEST,
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response = (self.status, self.body()).into_response();
        let headers = response.headers_mut();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        if self.status == StatusCode::UNAUTHORIZED {
            headers.insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        if self.status == StatusCode::SERVICE_UNAVAILABLE {
            headers.insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
        }
        response.extensions_mut().insert(ErrorCode(self.code));
        response
    }
}
