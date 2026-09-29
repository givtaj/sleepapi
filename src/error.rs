use axum::{
    Json,
    extract::rejection::JsonRejection,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use serde_json::json;

#[derive(Serialize)]
pub(crate) struct ErrorDetails {
    pub code: &'static str,
    pub message: String,
    pub retryable: bool,
}

impl ErrorDetails {
    pub fn for_status(status: StatusCode, message: impl Into<String>) -> Self {
        let code = match status.as_u16() {
            400 => "invalid_request",
            401 => "unauthorized",
            403 => "forbidden",
            404 => "not_found",
            405 => "method_not_allowed",
            408 => "request_timeout",
            409 => "conflict",
            413 => "payload_too_large",
            415 => "unsupported_media_type",
            422 => "validation_failed",
            429 => "rate_limited",
            500 => "internal_error",
            502 => "bad_gateway",
            503 => "service_unavailable",
            504 => "upstream_timeout",
            _ if status.is_client_error() => "client_error",
            _ => "server_error",
        };
        Self {
            code,
            message: message.into(),
            retryable: matches!(status.as_u16(), 408 | 429 | 500 | 502 | 503 | 504),
        }
    }

    pub fn simulated(status: StatusCode) -> Self {
        let message = match status.as_u16() {
            400 => "Simulated provider rejected the request.",
            401 => "Simulated provider requires valid credentials.",
            403 => "Simulated provider denied access.",
            404 => "Simulated provider could not find the resource.",
            405 => "Simulated provider only accepts GET for this resource.",
            408 => "Simulated provider timed out waiting for the request.",
            429 => "Simulated provider rate limit exceeded. Retry later.",
            503 => "Simulated provider is temporarily unavailable. Retry later.",
            504 => "Simulated gateway timed out waiting for its upstream provider.",
            _ => "Simulated provider returned an error.",
        };
        Self::for_status(status, message)
    }
}

pub(crate) fn validate_simulated_status(status_code: u16) -> Result<StatusCode, ApiError> {
    let reason = match status_code {
        204 | 205 | 304 => Some("cannot carry the JSON simulation response"),
        206 | 416 => Some("requires a range-response simulation, which is not supported"),
        226 => Some("requires a delta-encoding simulation, which is not supported"),
        407 => Some("requires a proxy-authentication simulation, which is not supported"),
        426 => Some("requires a protocol-upgrade simulation, which is not supported"),
        200..=599 => None,
        _ => {
            return Err(ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "status_code must be in 200..=599",
            ));
        }
    };
    if let Some(reason) = reason {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("status_code {status_code} {reason}"),
        ));
    }
    StatusCode::from_u16(status_code)
        .map_err(|_| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "invalid status_code"))
}

pub(crate) fn with_simulation_headers(response: Response) -> Response {
    let mut response = with_retry_after(response);
    match response.status() {
        StatusCode::UNAUTHORIZED => {
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_static("Bearer realm=\"sleepapi\""),
            );
        }
        StatusCode::METHOD_NOT_ALLOWED => {
            // This describes a mock provider, not the POST simulation route.
            response
                .headers_mut()
                .insert(header::ALLOW, HeaderValue::from_static("GET"));
        }
        _ => {}
    }
    response
}

pub(crate) fn with_retry_after(mut response: Response) -> Response {
    if matches!(response.status().as_u16(), 429 | 503) {
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
    }
    response
}

pub(crate) struct ApiError {
    status: StatusCode,
    error: ErrorDetails,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            error: ErrorDetails::for_status(status, message),
        }
    }

    pub fn capacity_exceeded() -> Self {
        Self {
            status: StatusCode::TOO_MANY_REQUESTS,
            error: ErrorDetails {
                code: "capacity_exceeded",
                message: "All simulation slots are busy; retry later.".into(),
                retryable: true,
            },
        }
    }
}

impl From<JsonRejection> for ApiError {
    fn from(error: JsonRejection) -> Self {
        Self::new(error.status(), error.body_text())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        with_retry_after(
            (
                self.status,
                Json(json!({
                    "simulated": false,
                    "status_code": self.status.as_u16(),
                    "error": self.error,
                })),
            )
                .into_response(),
        )
    }
}
