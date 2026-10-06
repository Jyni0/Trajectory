use serde::{Serialize, Serializer};

/// Unified error type for the whole core. Serialized to the frontend as
/// `{ kind, message }` so the UI can branch on the error category.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("secure storage error: {0}")]
    Secret(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("authentication failed: {0}")]
    Auth(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
}

impl AppError {
    pub fn kind(&self) -> &'static str {
        match self {
            AppError::Db(_) => "db",
            AppError::Secret(_) => "secret",
            AppError::Network(_) => "network",
            AppError::Auth(_) => "auth",
            AppError::Protocol(_) => "protocol",
            AppError::NotFound(_) => "not_found",
            AppError::Invalid(_) => "invalid",
            AppError::Io(_) => "io",
            AppError::Json(_) => "json",
        }
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("AppError", 2)?;
        s.serialize_field("kind", self.kind())?;
        s.serialize_field("message", &self.to_string())?;
        s.end()
    }
}

impl From<reqwest::Error> for AppError {
    fn from(e: reqwest::Error) -> Self {
        match e.status() {
            Some(s) if s.as_u16() == 401 || s.as_u16() == 403 => AppError::Auth(e.to_string()),
            Some(s) if s.as_u16() == 404 => AppError::NotFound(e.to_string()),
            _ => AppError::Network(e.to_string()),
        }
    }
}

impl From<async_imap::error::Error> for AppError {
    fn from(e: async_imap::error::Error) -> Self {
        use async_imap::error::Error as E;
        match e {
            E::Io(io) => AppError::Network(io.to_string()),
            E::No(msg) | E::Bad(msg) => AppError::Protocol(msg),
            other => AppError::Protocol(other.to_string()),
        }
    }
}

impl From<lettre::transport::smtp::Error> for AppError {
    fn from(e: lettre::transport::smtp::Error) -> Self {
        if e.is_permanent() {
            AppError::Auth(e.to_string())
        } else {
            AppError::Network(e.to_string())
        }
    }
}

impl From<lettre::error::Error> for AppError {
    fn from(e: lettre::error::Error) -> Self {
        AppError::Invalid(e.to_string())
    }
}

impl From<lettre::address::AddressError> for AppError {
    fn from(e: lettre::address::AddressError) -> Self {
        AppError::Invalid(format!("invalid address: {e}"))
    }
}

impl From<keyring::Error> for AppError {
    fn from(e: keyring::Error) -> Self {
        AppError::Secret(e.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
