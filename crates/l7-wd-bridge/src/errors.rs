use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BridgeError {
    #[error("bridge disabled")]
    Disabled,
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("no such session")]
    NoSuchSession,
    #[error("no such element")]
    NoSuchElement,
    #[error("invalid argument")]
    InvalidArgument,
    #[error("not implemented")]
    NotImplemented,
    #[error("internal error")]
    Internal,
}

pub type BridgeResult<T> = Result<T, BridgeError>;

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: ErrorBody,
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub code: &'static str,
    pub message: &'static str,
}

impl BridgeError {
    pub fn status_code(&self) -> StatusCode {
        match self {
            BridgeError::Disabled => StatusCode::SERVICE_UNAVAILABLE,
            BridgeError::Unauthorized => StatusCode::UNAUTHORIZED,
            BridgeError::Forbidden => StatusCode::FORBIDDEN,
            BridgeError::NoSuchSession => StatusCode::NOT_FOUND,
            BridgeError::NoSuchElement => StatusCode::NOT_FOUND,
            BridgeError::InvalidArgument => StatusCode::BAD_REQUEST,
            BridgeError::NotImplemented => StatusCode::NOT_IMPLEMENTED,
            BridgeError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            BridgeError::Disabled => "bridge_disabled",
            BridgeError::Unauthorized => "unauthorized",
            BridgeError::Forbidden => "forbidden",
            BridgeError::NoSuchSession => "no_such_session",
            BridgeError::NoSuchElement => "no_such_element",
            BridgeError::InvalidArgument => "invalid_argument",
            BridgeError::NotImplemented => "not_implemented",
            BridgeError::Internal => "internal",
        }
    }

    pub fn message(&self) -> &'static str {
        match self {
            BridgeError::Disabled => "webdriver bridge disabled",
            BridgeError::Unauthorized => "authorization required",
            BridgeError::Forbidden => "operation not allowed",
            BridgeError::NoSuchSession => "session not found",
            BridgeError::NoSuchElement => "element not found",
            BridgeError::InvalidArgument => "invalid argument",
            BridgeError::NotImplemented => "not implemented",
            BridgeError::Internal => "internal error",
        }
    }
}

impl IntoResponse for BridgeError {
    fn into_response(self) -> axum::response::Response {
        let status = self.status_code();
        let body = ErrorResponse {
            error: ErrorBody {
                code: self.code(),
                message: self.message(),
            },
        };
        (status, axum::Json(body)).into_response()
    }
}
